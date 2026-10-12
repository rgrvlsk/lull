/// LullKit: the macOS client of the lull daemon socket (contract §8, §10).
///
/// M0 ships only the protocol version the client speaks; the Codable models, the socket
/// client and the `@Observable` store arrive with WP4.1.
public enum LullKit {
    /// Protocol version this client speaks; must match `lull_protocol::PROTOCOL_VERSION`.
    public static let protocolVersion = ProtocolVersion(major: 1, minor: 1)
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
