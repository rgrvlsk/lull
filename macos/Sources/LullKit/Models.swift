// Codable mirrors of the wire types in crates/protocol (contract §4, §6, §7, §8).
// Unknown fields are ignored and string enums fall back to `.unknown`, so the app keeps
// working against a newer daemon of the same major (contract §8, "Versioning").
import Foundation

/// A string enum that decodes values it does not know as `.unknown`.
public protocol OpenEnum: RawRepresentable, Codable, Hashable, Sendable where RawValue == String {
    static var unknown: Self { get }
}

extension OpenEnum {
    public init(from decoder: any Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = Self(rawValue: raw) ?? .unknown
    }
}

public enum Framework: String, OpenEnum {
    case vite, next, astro, nuxt, remix, wrangler, artisan, rails, django, generic
    case phpBuiltin = "php-builtin", pythonHttp = "python-http"
    case unknown
}

public enum OriginKind: String, OpenEnum { case agent, user, unknown }
public enum ActivitySource: String, OpenEnum { case tab, traffic, started, manual, unknown }
public enum PinSource: String, OpenEnum { case env, user, unknown }
public enum StopReason: String, OpenEnum { case idle, manual, exited, unknown }
public enum RestartStatus: String, OpenEnum { case stopped, restarting, restartFailed, unknown }
public enum AutoStop: String, OpenEnum { case agent, all, none, unknown }
public enum TabsOnStop: String, OpenEnum { case discard, close, keep, unknown }
public enum ActivityBackend: String, OpenEnum { case ntstat, scan, unknown }
public enum ActivityHealth: String, OpenEnum { case healthy, failed, unknown }

public enum ServerState: String, OpenEnum {
    case running, stopping, unknown
    case stoppingSoon = "stopping-soon", noListener = "no-listener"
}

/// Who started a server (§4.5). `agent` is an open list: `claude`, `codex`, … or the
/// name of a configured `[[agent_marker]]`.
public struct Origin: Codable, Hashable, Sendable {
    public var kind: OriginKind
    public var agent: String?
    public var sessionId: String?

    public init(kind: OriginKind, agent: String? = nil, sessionId: String? = nil) {
        self.kind = kind
        self.agent = agent
        self.sessionId = sessionId
    }
}

/// The stopped-server record a restart came from, and who had started that server (§7).
public struct RestartedFrom: Codable, Hashable, Sendable {
    public var id: String
    public var origin: Origin

    public init(id: String, origin: Origin) {
        self.id = id
        self.origin = origin
    }
}

public struct TabRef: Codable, Hashable, Sendable {
    public var connectionId: String
    public var browser: String
    public var tabId: Int
    public var windowId: Int
}

/// A live server (§4 "Data model").
public struct Server: Codable, Hashable, Identifiable, Sendable {
    public var id: String
    public var pgid: Int
    public var pids: [Int]
    public var command: String
    public var argv: [String]
    public var cwd: String
    public var project: String
    public var framework: Framework
    public var ports: [Int]
    public var primaryUrl: String?
    public var origin: Origin
    public var startedAt: Date
    public var cpuPercent: Double
    public var rssBytes: Int64
    public var lastActivityAt: Date
    public var lastActivitySource: ActivitySource
    public var idleMinutes: Int
    public var autoStop: Bool
    public var pinned: Bool
    public var pinSource: PinSource?
    public var restartedFrom: RestartedFrom?
    public var state: ServerState
    public var stopsAt: Date?
    public var tabs: [TabRef]
}

/// A stopped-server record (§7.6).
public struct StoppedServer: Codable, Hashable, Identifiable, Sendable {
    public var id: String
    public var project: String
    public var framework: Framework
    public var command: String
    public var argv: [String]
    public var cwd: String
    public var ports: [Int]
    public var origin: Origin
    public var stoppedAt: Date
    public var reason: StopReason
    public var restartEnv: [String: String]
    public var tabs: [TabRef]
    public var status: RestartStatus
}

/// `config.get` / `config.set` (§6). Keys stay snake_case on the wire.
public struct Config: Codable, Hashable, Sendable {
    public var idleMinutes: Int
    public var autoStop: AutoStop
    public var warnBeforeMinutes: Int
    public var wakeGraceMinutes: Int
    public var tabsOnStop: TabsOnStop
    public var openWith: String
    public var historySize: Int
    public var historyHours: Int
    public var projects: [ProjectRule]?
    public var agentMarkers: [AgentMarker]?

    enum CodingKeys: String, CodingKey {
        case idleMinutes = "idle_minutes", autoStop = "auto_stop"
        case warnBeforeMinutes = "warn_before_minutes", wakeGraceMinutes = "wake_grace_minutes"
        case tabsOnStop = "tabs_on_stop", openWith = "open_with"
        case historySize = "history_size", historyHours = "history_hours"
        case projects = "project", agentMarkers = "agent_marker"
    }
}

public struct ProjectRule: Codable, Hashable, Sendable {
    public var path: String
    public var idleMinutes: Int?
    public var autoStop: AutoStop?

    public init(path: String, idleMinutes: Int? = nil, autoStop: AutoStop? = nil) {
        self.path = path
        self.idleMinutes = idleMinutes
        self.autoStop = autoStop
    }

    enum CodingKeys: String, CodingKey {
        case path, idleMinutes = "idle_minutes", autoStop = "auto_stop"
    }
}

public struct AgentMarker: Codable, Hashable, Sendable {
    public var env: String
    public var value: String?
    public var agent: String
}

/// `config.set {patch}`: only the fields that are set are sent; `projects` replaces the
/// whole list.
public struct ConfigPatch: Encodable, Sendable {
    public var idleMinutes: Int?
    public var autoStop: AutoStop?
    public var warnBeforeMinutes: Int?
    public var tabsOnStop: TabsOnStop?
    public var openWith: String?
    public var projects: [ProjectRule]?

    public init(
        idleMinutes: Int? = nil, autoStop: AutoStop? = nil, warnBeforeMinutes: Int? = nil,
        tabsOnStop: TabsOnStop? = nil, openWith: String? = nil, projects: [ProjectRule]? = nil
    ) {
        self.idleMinutes = idleMinutes
        self.autoStop = autoStop
        self.warnBeforeMinutes = warnBeforeMinutes
        self.tabsOnStop = tabsOnStop
        self.openWith = openWith
        self.projects = projects
    }

    enum CodingKeys: String, CodingKey {
        case idleMinutes = "idle_minutes", autoStop = "auto_stop"
        case warnBeforeMinutes = "warn_before_minutes", tabsOnStop = "tabs_on_stop"
        case openWith = "open_with", projects = "project"
    }
}

/// `daemon.status` (protocol 1.3).
public struct DaemonStatus: Codable, Hashable, Sendable {
    public var pid: Int
    public var version: String
    public var dryRun: Bool
    public var activityBackend: ActivityBackend
    public var activityHealth: ActivityHealth
    public var activityError: String?
    public var configError: String?
    public var warnings: [String]
    public var stops: [String]
    public var ineligible: [String: String]
    public var browsers: [BrowserConnection]
}

public struct BrowserConnection: Codable, Hashable, Sendable {
    public var connectionId: String
    public var name: String
    public var browserPid: Int?
    public var tabs: Int
}

public struct HelloParams: Encodable, Sendable {
    public var client = "app"
    public var version: String
    public var `protocol` = LullKit.protocolVersion
}

public struct HelloResult: Codable, Sendable {
    public var daemonVersion: String
    public var `protocol`: ProtocolVersion
    public var connectionId: String
    public var capabilities: [String]
    public var activityBackend: ActivityBackend
}

/// `error.data` of `-32010 protocol_mismatch`.
public struct ProtocolMismatch: Codable, Sendable {
    public var daemonVersion: String
    public var `protocol`: ProtocolVersion
}

public struct SubscribeResult: Codable, Sendable {
    public var seq: UInt64
    public var servers: [Server]
}

public struct OpenResult: Codable, Sendable {
    public var opened: Bool
    public var browser: String?
}

public struct RestartResult: Codable, Sendable {
    public var pid: Int
}

/// `{}`: params of methods without parameters, and results without content.
public struct Empty: Codable, Sendable {
    public init() {}
}

/// Daemon → client notifications (§8). Each carries the subscription `seq`.
public enum DaemonNotification: Decodable, Sendable {
    case added(seq: UInt64, server: Server)
    case updated(seq: UInt64, server: Server)
    case removed(seq: UInt64, id: String, reason: StopReason)

    public var seq: UInt64 {
        switch self {
        case .added(let seq, _), .updated(let seq, _), .removed(let seq, _, _): seq
        }
    }

    private struct Event: Decodable { var seq: UInt64; var server: Server }
    private struct Removed: Decodable { var seq: UInt64; var id: String; var reason: StopReason }
    private enum CodingKeys: String, CodingKey { case method, params }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(String.self, forKey: .method) {
        case "server.added":
            let e = try c.decode(Event.self, forKey: .params)
            self = .added(seq: e.seq, server: e.server)
        case "server.updated":
            let e = try c.decode(Event.self, forKey: .params)
            self = .updated(seq: e.seq, server: e.server)
        case "server.removed":
            let r = try c.decode(Removed.self, forKey: .params)
            self = .removed(seq: r.seq, id: r.id, reason: r.reason)
        case let method:
            throw DecodingError.dataCorruptedError(
                forKey: .method, in: c, debugDescription: "unknown notification \(method)")
        }
    }
}

extension JSONDecoder {
    /// Wire dates are RFC 3339 in UTC without fractional seconds.
    public static func lull() -> JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }
}
