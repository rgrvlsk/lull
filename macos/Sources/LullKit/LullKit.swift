import Foundation

/// LullKit: the macOS client of the lull daemon socket (contract §8, §10): Codable models
/// (`Models.swift`), one socket connection (`Connection`) and the observable `Store`.
public enum LullKit {
    /// Protocol version this client speaks; must match `lull_protocol::PROTOCOL_VERSION`.
    /// The app also needs a daemon of at least this minor.
    public static let protocolVersion = ProtocolVersion(major: 1, minor: 3)
}

/// Where the daemon keeps its socket and logs. `LULL_DIR` moves everything under one
/// directory, as for the CLI (tests, a second daemon); unset means the contract's places.
public enum LullPaths {
    private static var dir: String? { ProcessInfo.processInfo.environment["LULL_DIR"] }

    public static var socket: String {
        (dir ?? NSHomeDirectory() + "/Library/Application Support/lull") + "/daemon.sock"
    }

    /// `restart-<id>.log` of a stopped-server record (§7 Restart).
    public static func restartLog(_ recordID: String) -> URL {
        let logs = dir.map { $0 + "/logs" } ?? NSHomeDirectory() + "/Library/Logs"
        return URL(fileURLWithPath: logs + "/lull/restart-\(recordID).log")
    }
}

/// `hello.protocol` (contract §8).
public struct ProtocolVersion: Codable, Hashable, Sendable {
    public var major: Int
    public var minor: Int

    public init(major: Int, minor: Int) {
        self.major = major
        self.minor = minor
    }
}
