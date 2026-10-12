import { decodeMessage } from "./protocol.ts";
import type { BrowserCommand, BrowserInfo, BrowserTab, DaemonNotification, Method, ParamsByMethod, RequestId, ResultsByMethod, Server } from "./protocol.ts";

export const RECONNECT_ALARM = "lull-reconnect";
const ORIGIN_ERROR = "tab url no longer matches server origin";
export function loopbackUrl(value: string | undefined): URL | undefined {
  if (!value) return;
  try {
    const url = new URL(value);
    if ((url.protocol === "http:" || url.protocol === "https:")
        && (url.hostname === "localhost" || url.hostname === "127.0.0.1"
          || url.hostname === "[::1]" || url.hostname.endsWith(".localhost"))) return url;
  } catch { /* Incomplete navigation. */ }
}
export function browserInfo(version: string, agent: { userAgent: string; userAgentData?: { brands: readonly { brand: string; version: string }[] } }): BrowserInfo {
  const brands = agent.userAgentData?.brands.filter(({ brand }) => !/not.*brand/i.test(brand)) ?? [];
  const brand = brands.find(({ brand }) => brand.toLowerCase() !== "chromium") ?? brands[0];
  return { name: brand?.brand.toLowerCase() ?? "chromium", extensionVersion: version,
    version: brand?.version ?? agent.userAgent.match(/(?:Chrome|Chromium)\/([\d.]+)/)?.[1] ?? "unknown" };
}
interface Pending {
  method: Method; resolve: (value: unknown) => void; reject: (error: Error) => void;
  timeout: ReturnType<typeof setTimeout>;
}

// The daemon owns server binding, stopped records and port reuse (contract §9).
export function startBackground(api: typeof chrome, agent: Parameters<typeof browserInfo>[1]) {
  let port: chrome.runtime.Port | undefined;
  let ready = false;
  let synced = false;
  let seq: number | undefined;
  let subscribing = false;
  let nextId = 1;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let snapshots = Promise.resolve();
  let commands = Promise.resolve();
  let groupChanges = Promise.resolve();
  let focusedWindow: number | undefined;
  const pending = new Map<RequestId, Pending>();
  const servers = new Map<string, Server>();
  const origins = new Map<number, string>();
  const observedGroups = new Map<number, number>();
  const devGroups = new Set<number>();
  const grouping = new Set<number>();
  const buffered: DaemonNotification[] = [];
  // ponytail: Drag-out choices live for this worker only; use session storage if
  // preserving choices across worker/browser restarts becomes a requirement.
  const optedOut = new Set<number>();
  const visibility = new Map<number, object>();
  function badge(text: string, title = "lull") {
    void api.action.setBadgeText({ text });
    void api.action.setTitle({ title });
  }
  function requireConnection(connection: chrome.runtime.Port) {
    if (port !== connection) throw new Error("native host disconnected");
  }
  function drop(connection: chrome.runtime.Port) {
    if (port !== connection) return;
    port = undefined; ready = false; synced = false; seq = undefined; subscribing = false;
    servers.clear(); origins.clear(); observedGroups.clear(); devGroups.clear(); grouping.clear(); visibility.clear(); buffered.length = 0;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    for (const call of pending.values()) { clearTimeout(call.timeout); call.reject(new Error("native host disconnected")); }
    pending.clear(); badge("off");
  }
  function request<M extends Method>(method: M, params: ParamsByMethod[M]): Promise<ResultsByMethod[M]> {
    const connection = port;
    if (!connection) return Promise.reject(new Error("native host disconnected"));
    const id = nextId++;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => { drop(connection); connection.disconnect(); }, 10_000);
      pending.set(id, { method, resolve: (value) => resolve(value as ResultsByMethod[M]), reject, timeout });
      try { connection.postMessage({ jsonrpc: "2.0", id, method, params }); }
      catch { drop(connection); connection.disconnect(); }
    });
  }
  function groupTab(input: chrome.tabs.Tab, connection: chrome.runtime.Port) {
    const change = groupChanges.then(async () => {
      requireConnection(connection);
      const id = input.id;
      if (!api.tabGroups || id === undefined || optedOut.has(id)) return;
      // The tab may have moved windows while waiting for another grouping change.
      const tab = await api.tabs.get(id); requireConnection(connection);
      const current = loopbackUrl(tab.pendingUrl ?? tab.url);
      if (!current || current.origin !== loopbackUrl(input.url ?? input.pendingUrl)?.origin) return;
      const groups = await api.tabGroups.query({ windowId: tab.windowId, title: "Dev" });
      requireConnection(connection);
      if (optedOut.has(id)) return;
      for (const group of groups) devGroups.add(group.id);
      const group = groups[0];
      if (group && tab.groupId === group.id) return;
      grouping.add(id);
      try {
        const groupId = await api.tabs.group(group ? { tabIds: id, groupId: group.id }
          : { tabIds: id, createProperties: { windowId: tab.windowId } });
        requireConnection(connection);
        devGroups.add(groupId); observedGroups.set(id, groupId);
        if (!group) await api.tabGroups.update(groupId, { title: "Dev", color: "grey" });
      } finally { grouping.delete(id); }
    });
    groupChanges = change.catch(() => {});
    return change;
  }
  async function snapshot(connection: chrome.runtime.Port) {
    if (port !== connection || !ready) return;
    const windows = await api.windows.getAll(); requireConnection(connection);
    const focused = windows.find((window) => window.focused);
    if (focused?.id !== undefined) focusedWindow = focused.id;
    const focusedIds = new Set(windows.filter((window) => window.focused).map((window) => window.id));
    let tabs = await api.tabs.query({}); requireConnection(connection);
    if (api.tabGroups) {
      const groups = await api.tabGroups.query({ title: "Dev" }); requireConnection(connection);
      devGroups.clear(); for (const group of groups) devGroups.add(group.id);
    }
    const ports = new Set([...servers.values()].flatMap((server) => server.ports));
    for (const tab of tabs) {
      const url = loopbackUrl(tab.url);
      if (!url || tab.id === undefined) continue;
      if (!observedGroups.has(tab.id)) observedGroups.set(tab.id, tab.groupId);
      if (ports.has(Number(url.port || (url.protocol === "https:" ? 443 : 80)))) {
        try { await groupTab(tab, connection); }
        catch { requireConnection(connection); /* Tab closed or moved during adoption. */ }
      }
    }
    tabs = await api.tabs.query({}); requireConnection(connection);
    const reported: BrowserTab[] = [];
    const nextOrigins = new Map<number, string>();
    for (const tab of tabs) {
      const url = loopbackUrl(tab.url);
      if (!url || tab.id === undefined) continue;
      reported.push({ tabId: tab.id, windowId: tab.windowId, url: tab.url!, active: tab.active,
        windowFocused: focusedIds.has(tab.windowId), ...(tab.groupId >= 0 ? { groupId: tab.groupId } : {}) });
      nextOrigins.set(tab.id, url.origin);
    }
    // request posts synchronously; remember exactly the origins sent on this port.
    const sent = request("browser.tabs.sync", { tabs: reported });
    origins.clear(); for (const [id, origin] of nextOrigins) origins.set(id, origin);
    synced = true;
    await sent;
  }
  function syncNow(connection: chrome.runtime.Port) {
    snapshots = snapshots.then(() => snapshot(connection)).catch(() => {});
    return snapshots;
  }
  function scheduleSync() {
    if (!ready || !port) return;
    if (timer !== undefined) clearTimeout(timer);
    const connection = port;
    timer = setTimeout(() => { timer = undefined; void syncNow(connection); }, 100);
  }
  function notification(message: DaemonNotification, connection: chrome.runtime.Port) {
    if (subscribing) { buffered.push(message); return; }
    if (seq === undefined || message.params.seq <= seq) return;
    if (message.params.seq !== seq + 1) { void subscribe(connection); return; }
    seq = message.params.seq;
    if (message.method === "server.removed") { servers.delete(message.params.id); scheduleSync(); }
    else {
      const server = message.params.server;
      const old = servers.get(server.id);
      servers.set(server.id, server);
      // Activity/tab updates need no tab resync and must not create a feedback loop.
      if (!old || old.ports.join(",") !== server.ports.join(",")) scheduleSync();
    }
  }
  async function subscribe(connection: chrome.runtime.Port) {
    if (subscribing) return;
    subscribing = true;
    try {
      const result = await request("subscribe", { topics: ["servers"] }); requireConnection(connection);
      servers.clear(); for (const server of result.servers) servers.set(server.id, server);
      seq = result.seq; ready = true; subscribing = false; badge("");
      const queued = buffered.splice(0);
      for (const message of queued) notification(message, connection);
      await syncNow(connection);
    } catch { if (port === connection) { drop(connection); connection.disconnect(); } }
  }
  async function execute(command: BrowserCommand, reported: Map<number, string>, connection: chrome.runtime.Port) {
    requireConnection(connection);
    if (!ready || !synced) throw new Error("browser snapshot not ready");
    if (command.method === "tab.open") {
      if (!loopbackUrl(command.params.url)) throw new Error("url must be http(s) with a loopback or *.localhost host");
      let windowId = focusedWindow;
      if (windowId !== undefined) { try { await api.windows.get(windowId); } catch { windowId = undefined; } }
      if (windowId === undefined) windowId = (await api.windows.getLastFocused()).id;
      requireConnection(connection);
      const tab = await api.tabs.create({ url: command.params.url, active: command.params.focus, ...(windowId === undefined ? {} : { windowId }) });
      requireConnection(connection); await groupTab({ ...tab, url: command.params.url }, connection);
      if (command.params.focus) { requireConnection(connection); await api.windows.update(tab.windowId, { focused: true }); }
      scheduleSync(); return;
    }
    const ids = command.method === "tab.focus" ? [command.params.tabId] : command.params.tabIds;
    async function checkedTab(id: number) {
      let tab: chrome.tabs.Tab;
      try { tab = await api.tabs.get(id); }
      catch { throw new Error(ORIGIN_ERROR); }
      requireConnection(connection);
      const origin = loopbackUrl(tab.url)?.origin;
      if (!origin || reported.get(id) !== origin) throw new Error(ORIGIN_ERROR);
      return tab;
    }
    // Preflight the whole batch, then re-read immediately before each mutation.
    for (const id of ids) await checkedTab(id);
    for (const id of ids) {
      const tab = await checkedTab(id);
      switch (command.method) {
        case "tab.close": await api.tabs.remove(id); break;
        case "tab.discard":
          if (tab.active) throw new Error("active tab cannot be discarded");
          await api.tabs.discard(id); break;
        case "tab.reload": await api.tabs.reload(id); break;
        case "tab.focus":
          await api.tabs.update(id, { active: true }); requireConnection(connection);
          await api.windows.update(tab.windowId, { focused: true }); break;
      }
    }
    scheduleSync();
  }
  function receive(raw: unknown, connection: chrome.runtime.Port) {
    if (port !== connection) return;
    const id = typeof raw === "object" && raw !== null && "id" in raw ? raw.id : undefined;
    const call = typeof id === "number" || typeof id === "string" ? pending.get(id) : undefined;
    try {
      const message = decodeMessage(raw, call?.method);
      if (!("method" in message)) {
        if (!call) return;
        pending.delete(message.id); clearTimeout(call.timeout);
        if ("error" in message) call.reject(new Error(message.error.message)); else call.resolve(message.result);
      } else if ("id" in message) {
        if (!message.method.startsWith("tab.")) return;
        const reported = new Map(origins);
        commands = commands.then(async () => {
          let result: { ok: true } | { error: string };
          try { await execute(message as BrowserCommand, reported, connection); result = { ok: true }; }
          catch (error) { result = { error: error instanceof Error ? error.message : "tab command failed" }; }
          if (port === connection) connection.postMessage({ jsonrpc: "2.0", id: message.id, result });
        }).catch(() => {});
      } else notification(message, connection);
    } catch { drop(connection); connection.disconnect(); }
  }
  function connect() {
    if (port) return;
    try {
      const connection = api.runtime.connectNative("io.github.rgrvlsk.lull"); port = connection;
      connection.onMessage.addListener((message) => receive(message, connection));
      connection.onDisconnect.addListener(() => { void api.runtime.lastError; drop(connection); });
      const version = api.runtime.getManifest().version;
      void request("hello", { client: "browser", version, protocol: { major: 1, minor: 1 }, browser: browserInfo(version, agent) })
        .then((hello) => {
          requireConnection(connection);
          if (hello.protocol.major !== 1 || hello.protocol.minor < 1) throw new Error("update lull");
          return subscribe(connection);
        }).catch((error: unknown) => {
          if (port !== connection) return;
          drop(connection); connection.disconnect();
          if (error instanceof Error && ["protocol_mismatch", "update lull"].includes(error.message)) badge("updt", "update lull");
        });
    } catch { badge("off"); }
  }
  api.tabs.onCreated.addListener((tab) => { if (loopbackUrl(tab.url) || loopbackUrl(tab.pendingUrl)) scheduleSync(); });
  api.tabs.onUpdated.addListener((id, change, tab) => {
    if (!ready) return;
    const previous = observedGroups.get(id);
    if (change.groupId !== undefined) {
      if (previous !== undefined && devGroups.has(previous) && change.groupId !== previous && !grouping.has(id)) optedOut.add(id);
      observedGroups.set(id, change.groupId);
    }
    if ((change.url !== undefined || change.status !== undefined || change.groupId !== undefined)
        && (origins.has(id) || loopbackUrl(tab.url) || loopbackUrl(tab.pendingUrl))) scheduleSync();
  });
  api.tabs.onRemoved.addListener((id) => {
    if (origins.has(id)) scheduleSync();
    origins.delete(id); observedGroups.delete(id); optedOut.delete(id); visibility.delete(id);
  });
  api.tabs.onActivated.addListener(() => scheduleSync());
  api.windows.onFocusChanged.addListener((id) => { if (id !== api.windows.WINDOW_ID_NONE) focusedWindow = id; scheduleSync(); });
  api.runtime.onMessage.addListener((message: unknown, sender) => {
    if (!ready || !synced || !port || sender.tab?.id === undefined || sender.frameId !== 0
        || typeof message !== "object" || message === null || !("type" in message) || message.type !== "lull.visibility"
        || !("visible" in message) || typeof message.visible !== "boolean"
        || !("url" in message) || typeof message.url !== "string" || !loopbackUrl(message.url) || sender.url !== message.url) return;
    const id = sender.tab.id, token = {}, connection = port, url = message.url;
    visibility.set(id, token);
    if (!message.visible) return;
    void (async () => {
      if (await api.idle.queryState(60) !== "active") return;
      const tab = await api.tabs.get(id);
      if (port !== connection || !ready || visibility.get(id) !== token || tab.url !== url) return;
      await request("browser.activity", { tabId: id, url });
    })().catch(() => {});
  });
  api.alarms.onAlarm.addListener((alarm) => { if (alarm.name === RECONNECT_ALARM) connect(); });
  void api.alarms.create(RECONNECT_ALARM, { periodInMinutes: 0.5 });
  badge("off"); connect();
}
if (typeof chrome !== "undefined") startBackground(chrome, navigator);
