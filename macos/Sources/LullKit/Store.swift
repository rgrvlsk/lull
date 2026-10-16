// The app's view of the daemon: connect, `hello`, subscribe, apply notifications in `seq`
// order, and reconnect with backoff when the daemon goes away (contract §8, §10).
import Foundation
import Observation

@Observable @MainActor
public final class Store {
    public enum Phase: Equatable, Sendable {
        case connecting
        case connected
        case daemonNotRunning
        case protocolMismatch(daemonVersion: String)
    }

    public private(set) var phase = Phase.connecting
    /// Live servers by id.
    public private(set) var servers: [String: Server] = [:]
    /// Stopped-server records, newest first as the daemon sends them.
    public private(set) var history: [StoppedServer] = []
    public private(set) var status: DaemonStatus?
    public private(set) var daemonVersion: String?
    /// Why the last action failed; cleared by the next one.
    public var actionError: String?
    /// Called with (before, after) whenever a server appears, changes or goes away.
    @ObservationIgnored public var onServerChange: ((Server?, Server?) -> Void)?

    @ObservationIgnored private let socketPath: String
    @ObservationIgnored private var connection: Connection?
    @ObservationIgnored private var seq: UInt64 = 0
    @ObservationIgnored private var subscribing = false

    public init(socketPath: String = LullPaths.socket) {
        self.socketPath = socketPath
    }

    /// Live servers by project, then port.
    public var sortedServers: [Server] {
        servers.values.sorted {
            ($0.project, $0.ports.first ?? 0, $0.id) < ($1.project, $1.ports.first ?? 0, $1.id)
        }
    }

    // MARK: - Connection

    /// Runs until cancelled: connect, serve the session, back off (1 s doubling to 10 s,
    /// reset after a working session), repeat.
    public func run() async {
        var delay = Duration.seconds(1)
        while !Task.isCancelled {
            do {
                try await session(try Connection(path: socketPath))
                delay = .seconds(1)
                phase = .connecting
            } catch LullError.notRunning {
                phase = .daemonNotRunning
            } catch LullError.protocolMismatch(let version) {
                phase = .protocolMismatch(daemonVersion: version)
            } catch {
                phase = .connecting
            }
            for id in servers.keys { put(nil, id: id) }
            history = []
            status = nil
            try? await Task.sleep(for: delay)
            delay = min(delay * 2, .seconds(10))
        }
    }

    private func session(_ conn: Connection) async throws {
        defer { connection = nil }
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        let hello: HelloResult = try await conn.request("hello", HelloParams(version: version ?? "dev"))
        // This app needs what its own minor added (daemon.status, pinSource).
        guard hello.protocol.major == LullKit.protocolVersion.major,
              hello.protocol.minor >= LullKit.protocolVersion.minor
        else {
            await conn.close()
            throw LullError.protocolMismatch(daemonVersion: hello.daemonVersion)
        }
        connection = conn
        daemonVersion = hello.daemonVersion
        try await subscribe()
        phase = .connected
        await refresh()
        for await note in conn.notifications {
            apply(note)
        }
    }

    /// A full snapshot; it resets `seq` and replaces every server.
    private func subscribe() async throws {
        guard let connection, !subscribing else { return }
        subscribing = true
        defer { subscribing = false }
        struct Params: Encodable, Sendable { var topics = ["servers"] }
        let snapshot: SubscribeResult = try await connection.request("subscribe", Params())
        let fresh = Dictionary(snapshot.servers.map { ($0.id, $0) }, uniquingKeysWith: { $1 })
        for id in Set(servers.keys).union(fresh.keys) {
            put(fresh[id], id: id)
        }
        seq = snapshot.seq
    }

    /// The seq rule of §8: apply `seq + 1`, ignore what the snapshot already holds, and
    /// re-subscribe on a gap.
    func apply(_ note: DaemonNotification) {
        guard !subscribing, note.seq > seq else { return }
        guard note.seq == seq + 1 else {
            Task { try? await subscribe() }
            return
        }
        seq = note.seq
        switch note {
        case .added(_, let server):
            put(server, id: server.id)
            // A restart that succeeded drops its record (§7 Restart).
            Task { await refreshHistory() }
        case .updated(_, let server):
            put(server, id: server.id)
        case .removed(_, let id, _):
            put(nil, id: id)
            Task { await refreshHistory() }
        }
    }

    private func put(_ server: Server?, id: String) {
        let old = servers[id]
        guard old != server else { return }
        servers[id] = server
        onServerChange?(old, server)
    }

    // MARK: - Queries

    /// History and daemon status; also called when the panel opens.
    // ponytail: no history notification, so a record forgotten or restarted by another
    // client shows until the next server.added/removed or panel open. Upgrade: a
    // `history.changed` notification if stale records confuse anyone.
    public func refresh() async {
        await refreshHistory()
        if let status: DaemonStatus = await call("daemon.status") {
            self.status = status
        }
    }

    private func refreshHistory() async {
        if let history: [StoppedServer] = await call("servers.history") {
            self.history = history
        }
    }

    // MARK: - Actions

    private struct ID: Encodable, Sendable { var id: String }

    public func stop(_ id: String) async {
        let _: Empty? = await call("servers.stop", ID(id: id))
    }

    public func pin(_ id: String, _ pinned: Bool) async {
        struct Params: Encodable, Sendable { var id: String; var pinned: Bool }
        await update(call("servers.pin", Params(id: id, pinned: pinned)))
    }

    /// `nil` clears the per-server override.
    public func setAutoStop(_ id: String, _ autoStop: Bool?) async {
        struct Params: Encodable, Sendable { var id: String; var autoStop: Bool? }
        await update(call("servers.setAutoStop", Params(id: id, autoStop: autoStop)))
    }

    /// *Keep alive*: manual activity.
    public func touch(_ id: String) async {
        await update(call("servers.touch", ID(id: id)))
    }

    public func open(_ id: String) async {
        let _: OpenResult? = await call("servers.open", ID(id: id))
    }

    public func restart(_ recordID: String) async {
        let _: RestartResult? = await call("servers.restart", ID(id: recordID))
        await refreshHistory()
    }

    public func forget(_ recordID: String) async {
        if let _: Empty = await call("servers.forget", ID(id: recordID)) {
            history.removeAll { $0.id == recordID }
        }
    }

    public func config() async throws -> Config {
        try await connected().request("config.get")
    }

    /// Validation errors are thrown as `LullError.rpc` with the daemon's message.
    public func setConfig(_ patch: ConfigPatch) async throws -> Config {
        struct Params: Encodable, Sendable { var patch: ConfigPatch }
        return try await connected().request("config.set", Params(patch: patch))
    }

    /// The returned server is applied at once; the notification that follows matches it.
    private func update(_ server: Server?) {
        if let server { put(server, id: server.id) }
    }

    private func connected() throws -> Connection {
        guard let connection else { throw LullError.closed }
        return connection
    }

    private func call<R: Decodable & Sendable>(
        _ method: String, _ params: some Encodable & Sendable = Empty()
    ) async -> R? {
        do {
            let result: R = try await connected().request(method, params)
            actionError = nil
            return result
        } catch {
            actionError = error.localizedDescription
            return nil
        }
    }
}

#if DEBUG
extension Store {
    /// A store that never connects, for previews and the `LULL_PREVIEW` launch mode.
    public static func preview(
        phase: Phase = .connected, servers: [Server], history: [StoppedServer] = [],
        status: DaemonStatus? = nil
    ) -> Store {
        let store = Store(socketPath: "/dev/null")
        store.phase = phase
        store.servers = Dictionary(servers.map { ($0.id, $0) }, uniquingKeysWith: { $1 })
        store.history = history
        store.status = status
        return store
    }
}
#endif
