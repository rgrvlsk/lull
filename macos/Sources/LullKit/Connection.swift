// One connection to the daemon socket: JSON-RPC 2.0, one message per line (contract §8).
import Darwin
import Foundation

public enum LullError: Error, Equatable, Sendable {
    /// No daemon answers on the socket.
    case notRunning(String)
    /// The daemon speaks another major, or a minor older than this app needs.
    case protocolMismatch(daemonVersion: String)
    /// A JSON-RPC error; `message` is the daemon's, e.g. a `config.set` validation error.
    case rpc(code: Int, message: String)
    case closed
}

extension LullError: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .notRunning(let why): "The lull daemon is not running (\(why))."
        case .protocolMismatch(let v): "lull \(v) is too old or too new for this app; update lull."
        case .rpc(_, let message): message
        case .closed: "The connection to the lull daemon closed."
        }
    }
}

private struct Request<P: Encodable>: Encodable {
    var jsonrpc = "2.0"
    var id: Int
    var method: String
    var params: P
}

private struct Response<R: Decodable>: Decodable {
    var result: R
}

public actor Connection {
    private let handle: FileHandle
    private var nextID = 0
    private var pending: [Int: CheckedContinuation<Data, any Error>] = [:]
    private var open = true
    private let sink: AsyncStream<DaemonNotification>.Continuation
    /// Notifications in arrival order; finishes when the connection closes.
    public nonisolated let notifications: AsyncStream<DaemonNotification>

    /// Connects to the Unix socket at `path`.
    public init(path: String) throws {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw LullError.notRunning(String(cString: strerror(errno))) }
        var one: Int32 = 1
        // A write to a closed socket must fail, not raise SIGPIPE and end the app.
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
            Darwin.close(fd)
            throw LullError.notRunning("socket path too long")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: bytes) }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else {
            let why = String(cString: strerror(errno))
            Darwin.close(fd)
            throw LullError.notRunning(why)
        }
        handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
        (notifications, sink) = AsyncStream.makeStream()
        Task { await self.readLoop() }
    }

    /// One call; the daemon's error becomes `LullError.rpc` (or `.protocolMismatch`).
    public func request<R: Decodable & Sendable>(
        _ method: String, _ params: some Encodable & Sendable = Empty()
    ) async throws -> R {
        guard open else { throw LullError.closed }
        nextID += 1
        let id = nextID
        var line = try JSONEncoder().encode(Request(id: id, method: method, params: params))
        line.append(0x0A)
        let reply = try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            do {
                try handle.write(contentsOf: line)
            } catch {
                pending[id] = nil
                continuation.resume(throwing: LullError.closed)
            }
        }
        return try JSONDecoder.lull().decode(Response<R>.self, from: reply).result
    }

    /// Ends the connection; pending calls fail with `.closed`.
    public func close() {
        shutdown(handle.fileDescriptor, SHUT_RDWR)
    }

    private func readLoop() async {
        var line = Data()
        do {
            // Split on "\n" only: `AsyncLineSequence` also breaks on U+2028, which JSON
            // strings may contain unescaped.
            for try await byte in handle.bytes {
                if byte == 0x0A {
                    received(line)
                    line.removeAll(keepingCapacity: true)
                } else {
                    line.append(byte)
                }
            }
        } catch {}
        open = false
        for continuation in pending.values { continuation.resume(throwing: LullError.closed) }
        pending.removeAll()
        sink.finish()
    }

    private func received(_ line: Data) {
        struct RPCError: Decodable {
            var code: Int
            var message: String
            var data: ProtocolMismatch?
        }
        struct Incoming: Decodable {
            var id: Int?
            var method: String?
            var error: RPCError?
            enum CodingKeys: CodingKey { case id, method, error }
            init(from decoder: any Decoder) throws {
                let c = try decoder.container(keyedBy: CodingKeys.self)
                id = try? c.decodeIfPresent(Int.self, forKey: .id)  // daemon → browser ids are strings
                method = try c.decodeIfPresent(String.self, forKey: .method)
                error = try? c.decodeIfPresent(RPCError.self, forKey: .error)
            }
        }
        guard let msg = try? JSONDecoder().decode(Incoming.self, from: line) else { return }
        if msg.method != nil {
            // Notifications this client does not know are ignored (§8 "Versioning").
            if msg.id == nil, let note = try? JSONDecoder.lull().decode(DaemonNotification.self, from: line) {
                sink.yield(note)
            }
            return
        }
        guard let id = msg.id, let continuation = pending.removeValue(forKey: id) else { return }
        switch msg.error {
        case .some(let e) where e.code == -32010:
            continuation.resume(throwing: LullError.protocolMismatch(daemonVersion: e.data?.daemonVersion ?? "?"))
        case .some(let e):
            continuation.resume(throwing: LullError.rpc(code: e.code, message: e.message))
        case nil:
            continuation.resume(returning: line)
        }
    }
}
