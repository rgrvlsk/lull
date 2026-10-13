/// LullKit: the macOS client of the lull daemon socket (contract §8, §10): Codable models
/// (`Models.swift`), one socket connection (`Connection`) and the observable `Store`.
public enum LullKit {
    /// Protocol version this client speaks; must match `lull_protocol::PROTOCOL_VERSION`.
    /// The app also needs a daemon of at least this minor.
    public static let protocolVersion = ProtocolVersion(major: 1, minor: 3)
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
