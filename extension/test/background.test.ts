import { test } from "node:test";
import type { TestContext } from "node:test";
import assert from "node:assert/strict";
import { startBackground, RECONNECT_ALARM, loopbackUrl, browserInfo } from "../src/background.ts";
import { startContent } from "../src/content.ts";
import { fakeChrome, fixture } from "./fake-chrome.ts";
import type { Request, Response } from "../src/protocol.ts";
const agent = { userAgent: "Chrome/120.0", userAgentData: { brands: [{ brand: "Not(A:Brand", version: "99" }, { brand: "Chromium", version: "120" }, { brand: "Helium", version: "0.5.7.1" }] } };
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));
async function setup(t: TestContext, prepare?: (fake: ReturnType<typeof fakeChrome>) => void) {
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  const fake = fakeChrome(); prepare?.(fake); startBackground(fake.api, agent); await flush();
  return fake;
}
async function sync(t: TestContext) { t.mock.timers.tick(101); await flush(); }
function result(fake: ReturnType<typeof fakeChrome>, id: string) {
  return fake.ports.at(-1)!.sent.find((m): m is Response => !("method" in m) && m.id === id);
}

test("hello, subscribe, full loopback snapshot, existing/new per-window Dev groups", async (t) => {
  const fake = await setup(t, (f) => {
    f.groups.set(9, { id: 9, windowId: 3, title: "Dev", color: "blue", collapsed: false, shared: false });
    f.addTab(820, "http://foo.localhost:5187/", 4);
    f.addTab(821, "http://[::1]:5187/", 4);
  });
  const port = fake.ports[0]!;
  assert.deepEqual(port.sent.slice(0, 3).map((m) => "method" in m ? m.method : "ack"), ["hello", "subscribe", "browser.tabs.sync"]);
  assert.deepEqual((port.requests("hello")[0] as Request<"hello">).params, { client: "browser", version: "0.1.0", protocol: { major: 1, minor: 1 }, browser: { name: "helium", version: "0.5.7.1", extensionVersion: "0.1.0" } });
  assert.equal(fake.tabs.get(812)!.groupId, 9);
  assert.equal(fake.tabs.get(813)!.groupId, -1);
  assert.equal(fake.tabs.get(820)!.groupId, fake.tabs.get(821)!.groupId);
  assert.equal([...fake.groups.values()].find((g) => g.windowId === 4)!.title, "Dev");
  assert.equal([...fake.groups.values()].find((g) => g.windowId === 4)!.color, "grey");
  const snapshot = (port.requests("browser.tabs.sync")[0] as Request<"browser.tabs.sync">).params.tabs;
  assert.equal(snapshot.length, 5); assert.ok(snapshot.every((tab) => loopbackUrl(tab.url)));
  assert.equal(snapshot.find((tab) => tab.tabId === 820)!.windowFocused, false);
  assert.ok(fake.calls.some((call) => call.method === "badge" && (call.args[0] as { text: string }).text === ""));
});

test("fixture notifications update servers, gap re-subscribes once and adopts snapshot ports", async (t) => {
  const fake = await setup(t);
  const port = fake.ports[0]!;
  for (const message of fixture("subscribe").filter((m) => "method" in m && !("id" in m))) port.onMessage.emit(message);
  await flush(); await sync(t);
  assert.equal(port.requests("subscribe").length, 2);
  assert.notEqual(fake.tabs.get(813)!.groupId, -1);
  const updated = fixture("subscribe").find((m) => "method" in m && m.method === "server.updated")!;
  assert.ok("params" in updated && "server" in updated.params);
  port.onMessage.emit({ ...updated, params: { ...updated.params, seq: 47 } });
  await sync(t);
  const count = port.requests("browser.tabs.sync").length;
  port.onMessage.emit({ ...updated, params: { ...updated.params, seq: 48 } });
  await sync(t);
  assert.equal(port.requests("browser.tabs.sync").length, count, "activity-only server updates must not feed back into sync");
});

test("dragging out is remembered through later sync and reconnect; own grouping is not an opt-out", async (t) => {
  const fake = await setup(t);
  const tab = fake.tabs.get(812)!; tab.groupId = -1;
  fake.events.onUpdated.emit(812, { groupId: -1 }, tab);
  await sync(t);
  fake.events.onUpdated.emit(812, { status: "loading" }, tab); await sync(t);
  assert.equal(tab.groupId, -1);
  fake.ports[0]!.disconnect(); await flush();
  fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  assert.equal(tab.groupId, -1);
  fake.events.onRemoved.emit(812, { windowId: 3, isWindowClosing: false });
  fake.addTab(812, "http://127.0.0.1:5187/"); fake.events.onCreated.emit(fake.tabs.get(812)!); await sync(t);
  assert.notEqual(fake.tabs.get(812)!.groupId, -1, "closed IDs lose their opt-out");
});

test("tab changes debounce a fresh snapshot, external navigation drops the tab", async (t) => {
  const fake = await setup(t), port = fake.ports[0]!;
  fake.addTab(950, "http://other.localhost:5555/"); fake.events.onCreated.emit(fake.tabs.get(950)!);
  fake.events.onActivated.emit({ tabId: 950, windowId: 3 }); fake.events.onFocusChanged.emit(-1);
  await sync(t);
  assert.equal(port.requests("browser.tabs.sync").length, 2);
  const tab = fake.tabs.get(950)!; tab.url = "https://example.com/";
  fake.events.onUpdated.emit(950, { url: tab.url }, tab); await sync(t);
  const last = port.requests("browser.tabs.sync").at(-1) as Request<"browser.tabs.sync">;
  assert.ok(!last.params.tabs.some((t) => t.tabId === 950));
});

test("visibility heartbeat requires visible plus active idle state, without window focus", async (t) => {
  const fake = await setup(t), port = fake.ports[0]!;
  fake.tabs.get(812)!.windowId = 4;
  fake.visibility(812, false); await flush();
  fake.setIdle("idle"); fake.visibility(812, true); await flush();
  fake.setIdle("locked"); fake.visibility(812, true); await flush();
  assert.equal(port.requests("browser.activity").length, 0);
  fake.setIdle("active"); fake.visibility(812, true); await flush();
  assert.deepEqual((port.requests("browser.activity")[0] as Request<"browser.activity">).params, { tabId: 812, url: "http://127.0.0.1:5187/" });
  assert.ok(fake.calls.filter((c) => c.method === "idle").every((c) => c.args[0] === 60));
  fake.visibility(812, true, "http://127.0.0.1:5187/old-document"); await flush();
  assert.equal(port.requests("browser.activity").length, 1);
  fake.visibility(812, true); fake.visibility(812, false); await flush();
  assert.equal(port.requests("browser.activity").length, 1, "hidden report cancels in-flight heartbeat");
});

for (const [file, apiMethod] of [["tab-close", "remove"], ["tab-discard", "discard"], ["tab-reload", "reload"], ["tab-focus", "updateTab"]] as const) {
  test(`${file} executes fixture request and acknowledges its string ID`, async (t) => {
    const fake = await setup(t), port = fake.ports[0]!;
    const command = fixture(file)[0] as Request;
    port.onMessage.emit(command); await flush();
    assert.deepEqual(result(fake, command.id as string), fixture(file)[1]);
    assert.ok(fake.calls.some((c) => c.method === apiMethod));
  });
  test(`${file} refuses a changed origin before any tab mutation`, async (t) => {
    const fake = await setup(t), port = fake.ports[0]!;
    const command = fixture(file)[0] as Request;
    const id = file === "tab-reload" ? 640 : 812;
    fake.tabs.get(id)!.url = "http://localhost:9999/";
    port.onMessage.emit(command); await flush();
    assert.deepEqual(result(fake, command.id as string), { jsonrpc: "2.0", id: command.id, result: { error: "tab url no longer matches server origin" } });
    assert.ok(!fake.calls.some((c) => c.method === apiMethod));
  });
}

test("open routes to last focused window, groups, preserves background focus, and refuses remote URLs", async (t) => {
  const fake = await setup(t), port = fake.ports[0]!;
  fake.windows.get(3)!.focused = false; fake.windows.get(4)!.focused = true;
  // No focus event: use the browser's current state.
  fake.events.onFocusChanged.emit(-1);
  port.onMessage.emit(fixture("tab-open")[0]); await flush();
  assert.deepEqual(result(fake, "d1"), fixture("tab-open")[1]);
  assert.deepEqual(fake.calls.find((c) => c.method === "create")!.args, [{ url: "http://127.0.0.1:5187/admin", active: false, windowId: 4 }]);
  assert.equal(fake.calls.filter((c) => c.method === "updateWindow").length, 0);
  assert.equal([...fake.groups.values()].find((g) => g.windowId === 4)!.title, "Dev");
  port.onMessage.emit({ jsonrpc: "2.0", id: "focused", method: "tab.open", params: { url: "http://localhost:4321/", focus: true } }); await flush();
  assert.ok(fake.calls.some((c) => c.method === "updateWindow" && c.args[0] === 4));
  const count = fake.calls.filter((c) => c.method === "create").length;
  port.onMessage.emit({ jsonrpc: "2.0", id: "remote", method: "tab.open", params: { url: "https://example.com/", focus: true } }); await flush();
  assert.equal(fake.calls.filter((c) => c.method === "create").length, count);
  assert.ok("result" in result(fake, "remote")! && "error" in (result(fake, "remote") as Response & { result: { error: string } }).result);
});

test("same-origin navigation is allowed; missing and active tabs return error acks", async (t) => {
  const fake = await setup(t), port = fake.ports[0]!;
  fake.tabs.get(812)!.url += "admin";
  port.onMessage.emit(fixture("tab-discard")[0]); await flush();
  assert.deepEqual(result(fake, "d3"), fixture("tab-discard")[1]);
  fake.tabs.get(812)!.active = true;
  port.onMessage.emit({ jsonrpc: "2.0", id: "active", method: "tab.discard", params: { tabIds: [812] } }); await flush();
  assert.deepEqual(result(fake, "active"), { jsonrpc: "2.0", id: "active", result: { error: "active tab cannot be discarded" } });
  port.onMessage.emit(fixture("tab-focus")[2]); await flush();
  assert.deepEqual(result(fake, "d6"), fixture("tab-focus")[3]);
});

test("disconnect sets off badge, ignores tabs/activity, alarm reconnects with fresh state", async (t) => {
  const fake = await setup(t), old = fake.ports[0]!;
  old.disconnect(); await flush();
  assert.deepEqual(fake.calls.filter((c) => c.method === "badge").at(-1)!.args, [{ text: "off" }]);
  const count = fake.calls.filter((c) => c.method === "group").length;
  fake.addTab(955, "http://127.0.0.1:5187/"); fake.events.onCreated.emit(fake.tabs.get(955)!);
  fake.visibility(812, true); await sync(t);
  assert.equal(fake.calls.filter((c) => c.method === "group").length, count);
  assert.equal(old.requests("browser.activity").length, 0);
  fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  assert.equal(fake.ports.length, 2);
  assert.deepEqual(fake.ports[1]!.sent.slice(0, 3).map((m) => "method" in m ? m.method : "ack"), ["hello", "subscribe", "browser.tabs.sync"]);
  assert.notEqual(fake.tabs.get(955)!.groupId, -1);
  fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  assert.equal(fake.ports.length, 2);
  assert.ok(fake.calls.some((c) => c.method === "alarm" && JSON.stringify(c.args) === JSON.stringify([RECONNECT_ALARM, { periodInMinutes: 0.5 }])));
});

test("absent native host retries; no tabGroups degrades without losing sync or commands", async (t) => {
  const fake = await setup(t, (f) => { f.setAvailable(false); Reflect.deleteProperty(f.api, "tabGroups"); });
  assert.equal(fake.ports.length, 0); assert.ok(!fake.calls.some((c) => c.method === "group"));
  fake.setAvailable(true); fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  assert.equal(fake.ports[0]!.requests("browser.tabs.sync").length, 1);
  fake.ports[0]!.onMessage.emit(fixture("tab-open")[0]); await flush();
  assert.deepEqual(result(fake, "d1"), fixture("tab-open")[1]);
  assert.ok(!fake.calls.some((c) => c.method === "group"));
});

test("content script reports transitions and 30s visible heartbeats only", async (t) => {
  t.mock.timers.enable({ apis: ["setInterval"] });
  const sent: unknown[] = [], page = new EventTarget() as EventTarget & { visibilityState: DocumentVisibilityState };
  page.visibilityState = "visible";
  const runtime = { sendMessage: async (message: unknown) => { sent.push(message); } } as Pick<typeof chrome.runtime, "sendMessage">;
  const interval = startContent(runtime, page, { href: "http://localhost:5187/" });
  t.after(() => clearInterval(interval));
  assert.equal(sent.length, 1); t.mock.timers.tick(30_000); assert.equal(sent.length, 2);
  page.visibilityState = "hidden"; page.dispatchEvent(new Event("visibilitychange"));
  assert.equal(sent.length, 3); t.mock.timers.tick(60_000); assert.equal(sent.length, 3);
  page.visibilityState = "visible"; page.dispatchEvent(new Event("visibilitychange"));
  assert.deepEqual(sent.at(-1), { type: "lull.visibility", visible: true, url: "http://localhost:5187/" });
});

test("loopback recognition and browser identity include IPv6/subdomains and reject other schemes", () => {
  for (const url of ["http://localhost/", "https://127.0.0.1/", "http://[::1]:5187/", "https://app.localhost/"]) assert.ok(loopbackUrl(url));
  for (const url of ["file://localhost/a", "ftp://localhost/", "http://localhost.evil/", "https://example.com/", "bad"]) assert.equal(loopbackUrl(url), undefined);
  assert.equal(browserInfo("1", { userAgent: "Chrome/120.1" }).name, "chromium");
  assert.equal(browserInfo("1", agent).name, "helium");
});

test("protocol mismatch shows update lull; stalled handshake disconnects and retries", async (t) => {
  const fake = await setup(t, (f) => {
    const connect = f.api.runtime.connectNative;
    f.api.runtime.connectNative = (host) => {
      const port = connect(host) as unknown as (typeof f.ports)[number];
      port.helloError = { code: -32010, message: "protocol_mismatch", data: { daemonVersion: "2", protocol: { major: 2, minor: 0 } } };
      return port as unknown as chrome.runtime.Port;
    };
  });
  assert.equal(fake.ports[0]!.disconnected, true);
  assert.deepEqual(fake.calls.filter((c) => c.method === "title").at(-1)!.args, [{ title: "update lull" }]);
  assert.deepEqual(fake.calls.filter((c) => c.method === "badge").at(-1)!.args, [{ text: "updt" }]);
  const connect = fake.api.runtime.connectNative;
  fake.api.runtime.connectNative = (host) => {
    const port = connect(host) as unknown as (typeof fake.ports)[number];
    port.helloError = undefined; port.respond = false; return port as unknown as chrome.runtime.Port;
  };
  fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  t.mock.timers.tick(10_001); await flush();
  assert.equal(fake.ports[1]!.disconnected, true);
  assert.deepEqual(fake.calls.filter((c) => c.method === "badge").at(-1)!.args, [{ text: "off" }]);
});

test("notifications arriving with the subscribe response are retained", async (t) => {
  const fake = await setup(t, (f) => {
    const connect = f.api.runtime.connectNative;
    f.api.runtime.connectNative = (host) => {
      const port = connect(host) as unknown as (typeof f.ports)[number];
      const post = port.postMessage.bind(port);
      port.postMessage = (message) => {
        post(message);
        if ("method" in message && message.method === "subscribe") queueMicrotask(() => {
          port.onMessage.emit(fixture("subscribe").find((m) => "method" in m && m.method === "server.added"));
        });
      };
      return port as unknown as chrome.runtime.Port;
    };
  });
  assert.notEqual(fake.tabs.get(813)!.groupId, -1);
});

test("concurrent adoption and open create only one Dev group in a window", async (t) => {
  const fake = await setup(t), port = fake.ports[0]!;
  const query = fake.api.tabGroups.query.bind(fake.api.tabGroups);
  fake.api.tabGroups.query = (async (details: chrome.tabGroups.QueryInfo) => {
    const groups = await query(details);
    if (details.windowId === 4) await flush();
    return groups;
  }) as typeof fake.api.tabGroups.query;
  fake.windows.get(3)!.focused = false; fake.windows.get(4)!.focused = true;
  fake.addTab(820, "http://127.0.0.1:5187/", 4);
  fake.events.onCreated.emit(fake.tabs.get(820)!); fake.events.onFocusChanged.emit(4);
  port.onMessage.emit({ jsonrpc: "2.0", id: "race", method: "tab.open", params: { url: "http://localhost:5187/", focus: false } });
  t.mock.timers.tick(101);
  for (let i = 0; i < 8; i++) await flush();
  assert.deepEqual(result(fake, "race"), { jsonrpc: "2.0", id: "race", result: { ok: true } });
  assert.equal([...fake.groups.values()].filter((g) => g.windowId === 4 && g.title === "Dev").length, 1);
});

for (const url of ["https://example.com/", "http://localhost:9999/"]) {
  test(`queued adoption refuses a tab that has navigated to ${url}`, async (t) => {
    const fake = await setup(t, (f) => {
      const get = f.api.tabs.get.bind(f.api.tabs);
      f.api.tabs.get = (async (id: number) => {
        const tab = await get(id);
        if (id === 812) { tab.url = url; f.tabs.get(id)!.url = url; }
        return tab;
      }) as typeof f.api.tabs.get;
    });
    assert.equal(fake.tabs.get(812)!.groupId, -1);
  });
}

for (const url of ["https://example.com/", "http://localhost:9999/"]) {
  test(`adoption rechecks navigation to ${url} after the Dev-group lookup`, async (t) => {
    const fake = await setup(t, (f) => {
      const query = f.api.tabGroups.query.bind(f.api.tabGroups);
      f.api.tabGroups.query = (async (details: chrome.tabGroups.QueryInfo) => {
        const groups = await query(details);
        if (details.windowId === 3) f.tabs.get(812)!.url = url;
        return groups;
      }) as typeof f.api.tabGroups.query;
    });
    assert.equal(fake.tabs.get(812)!.groupId, -1);
    assert.ok(!fake.calls.some((c) => c.method === "group" && (c.args[0] as { tabIds: number }).tabIds === 812));
  });
}

test("dragging out while disconnected survives reconnect", async (t) => {
  const fake = await setup(t);
  fake.ports[0]!.disconnect(); await flush();
  const tab = fake.tabs.get(812)!; tab.groupId = -1;
  fake.events.onUpdated.emit(812, { groupId: -1 }, tab);
  fake.events.onAlarm.emit({ name: RECONNECT_ALARM, scheduledTime: 0 }); await flush();
  assert.equal(tab.groupId, -1);
  assert.equal(fake.ports[1]!.requests("browser.tabs.sync").length, 1);
});

for (const event of ["onDetached", "onAttached"] as const) {
  test(`${event} debounces a loopback tab snapshot with its current window`, async (t) => {
    const fake = await setup(t), port = fake.ports[0]!;
    const emit = (id: number) => {
      if (event === "onDetached") fake.events.onDetached.emit(id, { oldWindowId: 3, oldPosition: 0 });
      else fake.events.onAttached.emit(id, { newWindowId: 4, newPosition: 0 });
    };
    await sync(t); // Drain the initial adoption's group-change snapshot.
    const count = port.requests("browser.tabs.sync").length;
    emit(900); await flush(); await sync(t);
    assert.equal(port.requests("browser.tabs.sync").length, count);
    fake.tabs.get(812)!.windowId = 4;
    emit(812); emit(812); await flush(); await sync(t);
    assert.equal(port.requests("browser.tabs.sync").length, count + 1);
    const last = port.requests("browser.tabs.sync").at(-1) as Request<"browser.tabs.sync">;
    assert.equal(last.params.tabs.find((tab) => tab.tabId === 812)!.windowId, 4);
  });
}

for (const [accepted, urls] of [
  [true, ["http://localhost:5173/", "https://app.localhost", "http://127.0.0.1:8080/x?y", "http://127.0.0.2:5187/", "http://[::1]:3000/", "HTTP://LOCALHOST:1/"]],
  [false, ["https://example.com/", "file:///etc/passwd", "http://localhost@evil.com/", "http://user@localhost:5187/", "http://127.0.0.1.evil.com/", "http://localhost.evil.com/", "https://example.com\\.localhost/", "http://127.0.0.256/", "http://app%2elocalhost/", "http://user:secret@localhost/"]],
] as const) {
  for (const url of urls) test(`daemon loopback rule ${accepted ? "accepts" : "rejects"} ${url}`, () => {
    assert.equal(loopbackUrl(url) !== undefined, accepted);
  });
}
