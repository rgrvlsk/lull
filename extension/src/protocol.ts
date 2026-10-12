// Wire names and shapes mirror crates/protocol. No URLs are fetched by the extension.
export type RequestId = number | string;
export interface ProtocolVersion { major: number; minor: number }
export interface BrowserInfo { name: string; profile?: string; version: string; extensionVersion: string }
export interface HelloParams { client: "app" | "cli" | "browser"; version: string; protocol: ProtocolVersion; browser?: BrowserInfo }
export interface HelloResult { daemonVersion: string; protocol: ProtocolVersion; connectionId: string; capabilities: string[]; activityBackend: "ntstat" | "scan" }
export type Framework = "vite" | "next" | "astro" | "nuxt" | "remix" | "wrangler" | "php-builtin" | "artisan" | "python-http" | "rails" | "django" | "generic";
export type StopReason = "idle" | "manual" | "exited";
export interface Origin { kind: "agent" | "user"; agent?: string; sessionId?: string }
export interface TabRef { connectionId: string; browser: string; tabId: number; windowId: number }
export interface Server {
  id: string; pgid: number; pids: number[]; command: string; argv: string[]; cwd: string;
  project: string; framework: Framework; ports: number[]; primaryUrl: string | null; origin: Origin;
  startedAt: string; cpuPercent: number; rssBytes: number; lastActivityAt: string;
  lastActivitySource: "tab" | "traffic" | "started" | "manual"; idleMinutes: number;
  autoStop: boolean; pinned: boolean; state: "running" | "stopping-soon" | "stopping" | "no-listener";
  stopsAt: string | null; tabs: TabRef[];
}
export interface StoppedServer {
  id: string; project: string; framework: Framework; command: string; argv: string[]; cwd: string;
  ports: number[]; origin: Origin; stoppedAt: string; reason: StopReason; restartEnv: Record<string, string>;
  tabs: TabRef[]; status: "stopped" | "restarting" | "restartFailed";
}
export interface BrowserTab { tabId: number; windowId: number; url: string; groupId?: number | null; active: boolean; windowFocused: boolean }
export interface TabsSyncParams { tabs: BrowserTab[] }
export interface BrowserActivityParams { tabId: number; url: string }
export interface SubscribeParams { topics: ("servers" | "browsers")[] }
export interface SubscribeResult { seq: number; servers: Server[] }
export interface ServerEvent { seq: number; server: Server }
export interface ServerRemoved { seq: number; id: string; reason: StopReason }
export type Ack = { ok: boolean } | { error: string };
export interface ProjectRule { path: string; idle_minutes?: number; auto_stop?: "agent" | "all" | "none" }
export interface AgentMarker { env: string; value?: string; agent: string }
export interface Config {
  idle_minutes: number; auto_stop: "agent" | "all" | "none"; warn_before_minutes: number;
  wake_grace_minutes: number; tabs_on_stop: "discard" | "close" | "keep"; open_with: string;
  history_size: number; history_hours: number; project?: ProjectRule[]; agent_marker?: AgentMarker[];
}
export type ConfigPatch = Partial<Config>;
export interface ParamsByMethod {
  hello: HelloParams;
  "servers.list": Record<string, never>;
  "servers.stop": { id: string };
  "servers.pin": { id: string; pinned: boolean };
  "servers.setAutoStop": { id: string; autoStop: boolean | null };
  "servers.touch": { id: string };
  "servers.history": Record<string, never>;
  "servers.restart": { id: string };
  "servers.forget": { id: string };
  "servers.open": { id: string; path?: string; focus?: boolean } | { url: string; focus?: boolean };
  "config.get": Record<string, never>;
  "config.set": { patch: ConfigPatch };
  subscribe: SubscribeParams;
  "browser.tabs.sync": TabsSyncParams;
  "browser.activity": BrowserActivityParams;
  "tab.open": { url: string; focus: boolean };
  "tab.close": { tabIds: number[] };
  "tab.discard": { tabIds: number[] };
  "tab.reload": { tabIds: number[] };
  "tab.focus": { tabId: number };
}
export interface ResultsByMethod {
  hello: HelloResult;
  "servers.list": Server[];
  "servers.stop": Record<string, never>;
  "servers.pin": Server;
  "servers.setAutoStop": Server;
  "servers.touch": Server;
  "servers.history": StoppedServer[];
  "servers.restart": { pid: number };
  "servers.forget": Record<string, never>;
  "servers.open": { opened: boolean; browser?: string };
  "config.get": Config;
  "config.set": Config;
  subscribe: SubscribeResult;
  "browser.tabs.sync": Record<string, never>;
  "browser.activity": Record<string, never>;
  "tab.open": Ack; "tab.close": Ack; "tab.discard": Ack; "tab.reload": Ack; "tab.focus": Ack;
}
export type Method = keyof ParamsByMethod;
export type Request<M extends Method = Method> = { [K in M]: { jsonrpc: "2.0"; id: RequestId; method: K; params: ParamsByMethod[K] } }[M];
export type BrowserCommand = Request<"tab.open" | "tab.close" | "tab.discard" | "tab.reload" | "tab.focus">;
export type DaemonNotification =
  | { jsonrpc: "2.0"; method: "server.added" | "server.updated"; params: ServerEvent }
  | { jsonrpc: "2.0"; method: "server.removed"; params: ServerRemoved };
export interface RpcError { code: number; message: string; data?: unknown }
export type Response<M extends Method = Method> = { jsonrpc: "2.0"; id: RequestId } & ({ result: ResultsByMethod[M] } | { error: RpcError });
export type Message = Request | DaemonNotification | Response;

// Small structural guards. Object guards reject extra fields, as Rust's serde does.
type Guard<T> = (value: unknown) => value is T;
const string: Guard<string> = (v): v is string => typeof v === "string";
const boolean: Guard<boolean> = (v): v is boolean => typeof v === "boolean";
const number: Guard<number> = (v): v is number => typeof v === "number" && Number.isFinite(v);
const integer: Guard<number> = (v): v is number => number(v) && Number.isInteger(v);
const uint = (max: number): Guard<number> => (v): v is number => integer(v) && v >= 0 && v <= max;
const u32 = uint(0xffffffff);
// JSON numbers use IEEE-754 in JS and Rust serde_json's f64 consumer alike; never use
// counters beyond JS's safe range. Daemon seq would need ~285 million years at 1 Hz.
const u64 = uint(Number.MAX_SAFE_INTEGER);
const literal = <T extends string>(...values: T[]): Guard<T> => (v): v is T => string(v) && values.includes(v as T);
const array = <T>(guard: Guard<T>): Guard<T[]> => (v): v is T[] => Array.isArray(v) && v.every(guard);
const optional = <T>(guard: Guard<T>): Guard<T | undefined> => (v): v is T | undefined => v === undefined || guard(v);
const nullable = <T>(guard: Guard<T>): Guard<T | null> => (v): v is T | null => v === null || guard(v);
const record = <T>(guard: Guard<T>): Guard<Record<string, T>> => (v): v is Record<string, T> => isObject(v) && Object.values(v).every(guard);
const either = <A, B>(a: Guard<A>, b: Guard<B>): Guard<A | B> => (v): v is A | B => a(v) || b(v);
function isObject(v: unknown): v is Record<string, unknown> { return typeof v === "object" && v !== null && !Array.isArray(v); }
function object<T>(fields: { [K in keyof T]-?: Guard<T[K]> }): Guard<T> {
  return (v): v is T => isObject(v)
    && Object.keys(v).every((key) => Object.hasOwn(fields, key))
    && Object.entries(fields).every(([key, guard]) => (guard as Guard<unknown>)(v[key]));
}
const empty = object<Record<string, never>>({});
const protocol = object<ProtocolVersion>({ major: u32, minor: u32 });
const browser = object<BrowserInfo>({ name: string, profile: optional(string), version: string, extensionVersion: string });
const hello = object<HelloParams>({ client: literal("app", "cli", "browser"), version: string, protocol, browser: optional(browser) });
const helloResult = object<HelloResult>({ daemonVersion: string, protocol, connectionId: string, capabilities: array(string), activityBackend: literal("ntstat", "scan") });
const framework = literal<Framework>("vite", "next", "astro", "nuxt", "remix", "wrangler", "php-builtin", "artisan", "python-http", "rails", "django", "generic");
const origin = object<Origin>({ kind: literal("agent", "user"), agent: optional(string), sessionId: optional(string) });
const tabRef = object<TabRef>({ connectionId: string, browser: string, tabId: integer, windowId: integer });
const reason = literal<StopReason>("idle", "manual", "exited");
const server = object<Server>({
  id: string, pgid: u32, pids: array(u32), command: string, argv: array(string), cwd: string,
  project: string, framework, ports: array(uint(65535)), primaryUrl: nullable(string), origin,
  startedAt: string, cpuPercent: number, rssBytes: u64, lastActivityAt: string,
  lastActivitySource: literal("tab", "traffic", "started", "manual"), idleMinutes: u32,
  autoStop: boolean, pinned: boolean, state: literal("running", "stopping-soon", "stopping", "no-listener"),
  stopsAt: nullable(string), tabs: array(tabRef),
});
const stopped = object<StoppedServer>({
  id: string, project: string, framework, command: string, argv: array(string), cwd: string,
  ports: array(uint(65535)), origin, stoppedAt: string, reason, restartEnv: record(string),
  tabs: array(tabRef), status: literal("stopped", "restarting", "restartFailed"),
});
const autoStop = literal("agent", "all", "none");
const project = object<ProjectRule>({ path: string, idle_minutes: optional(u32), auto_stop: optional(autoStop) });
const marker = object<AgentMarker>({ env: string, value: optional(string), agent: string });
const configFields = {
  idle_minutes: u32, auto_stop: autoStop, warn_before_minutes: u32, wake_grace_minutes: u32,
  tabs_on_stop: literal("discard", "close", "keep"), open_with: string, history_size: u32,
  history_hours: u32, project: optional(array(project)), agent_marker: optional(array(marker)),
};
const config = object<Config>(configFields);
const patch = object<ConfigPatch>(Object.fromEntries(Object.entries(configFields).map(([key, guard]) => [key, optional(guard as Guard<unknown>)])) as { [K in keyof Config]-?: Guard<Config[K] | undefined> });
const id = object<{ id: string }>({ id: string });
const tabIds = object<{ tabIds: number[] }>({ tabIds: array(integer) });
const ack = either(object<{ ok: boolean }>({ ok: boolean }), object<{ error: string }>({ error: string }));
const params: { [M in Method]: Guard<ParamsByMethod[M]> } = {
  hello, "servers.list": empty, "servers.stop": id, "servers.pin": object({ id: string, pinned: boolean }),
  "servers.setAutoStop": object({ id: string, autoStop: nullable(boolean) }), "servers.touch": id,
  "servers.history": empty, "servers.restart": id, "servers.forget": id,
  "servers.open": either(object<{ id: string; path?: string; focus?: boolean }>({ id: string, path: optional(string), focus: optional(boolean) }), object<{ url: string; focus?: boolean }>({ url: string, focus: optional(boolean) })),
  "config.get": empty, "config.set": object({ patch }), subscribe: object({ topics: array(literal("servers", "browsers")) }),
  "browser.tabs.sync": object({ tabs: array(object<BrowserTab>({ tabId: integer, windowId: integer, url: string, groupId: optional(nullable(integer)), active: boolean, windowFocused: boolean })) }),
  "browser.activity": object({ tabId: integer, url: string }),
  "tab.open": object({ url: string, focus: boolean }), "tab.close": tabIds, "tab.discard": tabIds,
  "tab.reload": tabIds, "tab.focus": object({ tabId: integer }),
};
const results: { [M in Method]: Guard<ResultsByMethod[M]> } = {
  hello: helloResult, "servers.list": array(server), "servers.stop": empty, "servers.pin": server,
  "servers.setAutoStop": server, "servers.touch": server, "servers.history": array(stopped),
  "servers.restart": object({ pid: u32 }), "servers.forget": empty, "servers.open": object<{ opened: boolean; browser?: string }>({ opened: boolean, browser: optional(string) }),
  "config.get": config, "config.set": config, subscribe: object({ seq: u64, servers: array(server) }),
  "browser.tabs.sync": empty, "browser.activity": empty,
  "tab.open": ack, "tab.close": ack, "tab.discard": ack, "tab.reload": ack, "tab.focus": ack,
};
const notifications = {
  "server.added": object<ServerEvent>({ seq: u64, server }),
  "server.updated": object<ServerEvent>({ seq: u64, server }),
  "server.removed": object<ServerRemoved>({ seq: u64, id: string, reason }),
};
export function decodeMessage(value: unknown, responseMethod?: Method): Message {
  if (!isObject(value) || value.jsonrpc !== "2.0") throw new Error("invalid JSON-RPC envelope");
  const validId = either(integer, string);
  if (typeof value.method === "string") {
    if (value.id !== undefined) {
      if (!validId(value.id) || !Object.hasOwn(params, value.method)
          || !params[value.method as Method](value.params ?? {})
          || Object.keys(value).some((k) => !["jsonrpc", "id", "method", "params"].includes(k))) throw new Error("invalid request");
    } else if (!Object.hasOwn(notifications, value.method)
        || !notifications[value.method as keyof typeof notifications](value.params ?? {})
        || Object.keys(value).some((k) => !["jsonrpc", "method", "params"].includes(k))) throw new Error("invalid notification");
  } else {
    if (!validId(value.id) || (Object.hasOwn(value, "result") === Object.hasOwn(value, "error"))
        || Object.keys(value).some((k) => !["jsonrpc", "id", "result", "error"].includes(k))) throw new Error("invalid response");
    if (Object.hasOwn(value, "error")) {
      if (!object<RpcError>({ code: integer, message: string, data: optional(((_): _ is unknown => true)) })(value.error)) throw new Error("invalid RPC error");
      if (value.error.code === -32010 && !object({ daemonVersion: string, protocol })(value.error.data)) throw new Error("invalid protocol mismatch");
    } else if (!responseMethod || !results[responseMethod](value.result)) throw new Error("invalid result");
  }
  return value as Message;
}
