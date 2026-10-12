import { readFileSync } from "node:fs";
import { decodeMessage } from "../src/protocol.ts";
import type { Message, Request, Response, SubscribeResult } from "../src/protocol.ts";

export function fixture(file: string): Message[] {
  const pending = new Map<number | string, Request["method"]>();
  return readFileSync(new URL(`../../protocol/fixtures/${file}.ndjson`, import.meta.url), "utf8").trim().split("\n").map((line) => {
    const raw = JSON.parse(line) as { id?: number | string };
    const message = decodeMessage(raw, raw.id === undefined ? undefined : pending.get(raw.id));
    if ("method" in message && "id" in message) pending.set(message.id, message.method);
    return message;
  });
}
export class Event<Args extends unknown[]> {
  listeners: ((...args: Args) => void)[] = [];
  addListener(listener: (...args: Args) => void) { this.listeners.push(listener); }
  emit(...args: Args) { for (const listener of this.listeners) listener(...args); }
}
export class NativePort {
  onMessage = new Event<[unknown]>();
  onDisconnect = new Event<[]>();
  sent: (Request | Response)[] = [];
  disconnected = false;
  respond = true;
  helloError: { code: number; message: string; data: unknown } | undefined;
  subscriptions = 0;
  subscribeResults: SubscribeResult[] = fixture("subscribe").filter((m): m is Response<"subscribe"> & { result: SubscribeResult } => "result" in m).map((m) => m.result);
  postMessage(message: Request | Response) {
    this.sent.push(structuredClone(message));
    if (!("method" in message) || !this.respond) return;
    const reply = () => {
      if (this.disconnected) return;
      if (message.method === "hello" && this.helloError) {
        this.onMessage.emit({ jsonrpc: "2.0", id: message.id, error: this.helloError }); return;
      }
      let result: unknown;
      if (message.method === "hello") result = (fixture("hello")[1] as Response<"hello"> & { result: unknown }).result;
      else if (message.method === "subscribe") result = this.subscribeResults[Math.min(this.subscriptions++, this.subscribeResults.length - 1)];
      else result = {};
      this.onMessage.emit({ jsonrpc: "2.0", id: message.id, result });
    };
    queueMicrotask(reply);
  }
  disconnect() { this.disconnected = true; this.onDisconnect.emit(); }
  requests(method: string) { return this.sent.filter((m): m is Request => "method" in m && m.method === method); }
}
export function fakeChrome() {
  const tabs = new Map<number, chrome.tabs.Tab>();
  function addTab(id: number, url: string, windowId = 3, active = false, groupId = -1) {
    const tab = { id, url, windowId, active, groupId, index: 0, highlighted: active, pinned: false, incognito: false, selected: active, discarded: false, frozen: false, autoDiscardable: true };
    tabs.set(id, tab); return tab;
  }
  addTab(812, "http://127.0.0.1:5187/"); addTab(813, "http://localhost:4321/about"); addTab(640, "http://localhost:3000/"); addTab(900, "https://example.com/");
  const groups = new Map<number, chrome.tabGroups.TabGroup>();
  const windows = new Map<number, chrome.windows.Window>([
    [3, { id: 3, focused: true, alwaysOnTop: false, incognito: false }],
    [4, { id: 4, focused: false, alwaysOnTop: false, incognito: false }],
  ]);
  const calls: { method: string; args: unknown[] }[] = [];
  const ports: NativePort[] = [];
  const onCreated = new Event<[chrome.tabs.Tab]>();
  const onUpdated = new Event<[number, chrome.tabs.OnUpdatedInfo, chrome.tabs.Tab]>();
  const onRemoved = new Event<[number, { windowId: number; isWindowClosing: boolean }]>();
  const onAttached = new Event<[number, chrome.tabs.OnAttachedInfo]>();
  const onDetached = new Event<[number, chrome.tabs.OnDetachedInfo]>();
  const onActivated = new Event<[{ tabId: number; windowId: number }]>();
  const onFocusChanged = new Event<[number]>();
  const onMessage = new Event<[unknown, chrome.runtime.MessageSender]>();
  const onAlarm = new Event<[chrome.alarms.Alarm]>();
  let nextGroup = 17, nextTab = 1000;
  let idleState: "active" | "idle" | "locked" = "active";
  let available = true;
  function record(method: string, ...args: unknown[]) { calls.push({ method, args: structuredClone(args) }); }
  function getTab(id: number) { const tab = tabs.get(id); if (!tab) throw new Error("No tab with id"); return tab; }
  const api = {
    action: { setBadgeText: async (details: unknown) => { record("badge", details); }, setTitle: async (details: unknown) => { record("title", details); } },
    runtime: {
      getManifest: () => ({ version: "0.1.0" }), onMessage,
      connectNative: (host: string) => { record("connectNative", host); if (!available) throw new Error("Native host missing"); const port = new NativePort(); ports.push(port); return port; },
    },
    tabs: {
      onCreated, onUpdated, onRemoved, onAttached, onDetached, onActivated,
      query: async () => [...tabs.values()].map((tab) => structuredClone(tab)),
      get: async (id: number) => structuredClone(getTab(id)),
      create: async (details: { url: string; active: boolean; windowId?: number }) => {
        record("create", details); const tab = addTab(nextTab++, details.url, details.windowId ?? 3, details.active);
        onCreated.emit(tab); return structuredClone(tab);
      },
      group: async (details: { tabIds: number; groupId?: number; createProperties?: { windowId: number } }) => {
        record("group", details); const tab = getTab(details.tabIds), groupId = details.groupId ?? nextGroup++;
        if (!groups.has(groupId)) groups.set(groupId, { id: groupId, windowId: details.createProperties?.windowId ?? tab.windowId, title: "", color: "grey", collapsed: false, shared: false });
        tab.groupId = groupId; onUpdated.emit(tab.id!, { groupId }, tab); return groupId;
      },
      remove: async (id: number) => { record("remove", id); const tab = getTab(id); tabs.delete(id); onRemoved.emit(id, { windowId: tab.windowId, isWindowClosing: false }); },
      discard: async (id: number) => { record("discard", id); const tab = getTab(id); if (tab.active) throw new Error("Cannot discard active tab"); tab.discarded = true; return tab; },
      reload: async (id: number) => { record("reload", id); const tab = getTab(id); onUpdated.emit(id, { status: "loading" }, tab); },
      update: async (id: number, details: { active: boolean }) => {
        record("updateTab", id, details); const tab = getTab(id);
        for (const candidate of tabs.values()) if (candidate.windowId === tab.windowId) candidate.active = candidate.id === id;
        onActivated.emit({ tabId: id, windowId: tab.windowId }); return tab;
      },
    },
    tabGroups: {
      query: async (details: { windowId?: number; title?: string }) => [...groups.values()].filter((g) => (details.windowId === undefined || g.windowId === details.windowId) && (details.title === undefined || g.title === details.title)),
      update: async (id: number, details: { title: string; color: chrome.tabGroups.TabGroup["color"] }) => { record("updateGroup", id, details); const group = groups.get(id)!; Object.assign(group, details); return group; },
    },
    windows: {
      WINDOW_ID_NONE: -1, onFocusChanged,
      getAll: async () => [...windows.values()].map((w) => structuredClone(w)),
      get: async (id: number) => { const w = windows.get(id); if (!w) throw new Error("Window closed"); return structuredClone(w); },
      getLastFocused: async () => structuredClone([...windows.values()].find((w) => w.focused) ?? windows.get(3)!),
      update: async (id: number, details: { focused: boolean }) => {
        record("updateWindow", id, details); for (const w of windows.values()) w.focused = w.id === id;
        onFocusChanged.emit(id); return windows.get(id)!;
      },
    },
    idle: { queryState: async (seconds: number) => { record("idle", seconds); return idleState; } },
    alarms: { onAlarm, create: async (...args: unknown[]) => { record("alarm", ...args); } },
  };
  return { api: api as unknown as typeof chrome, events: { onCreated, onUpdated, onRemoved, onAttached, onDetached, onActivated, onFocusChanged, onMessage, onAlarm },
    tabs, groups, windows, calls, ports, addTab,
    setIdle: (state: "active" | "idle" | "locked") => { idleState = state; },
    setAvailable: (value: boolean) => { available = value; },
    visibility: (id: number, visible: boolean, url = getTab(id).url!) => onMessage.emit({ type: "lull.visibility", visible, url }, { tab: structuredClone(getTab(id)), url, frameId: 0 }),
  };
}
