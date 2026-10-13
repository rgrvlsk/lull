// A scripted daemon on a temporary Unix socket: it answers each request through
// `respond` and can push notification lines to the connected client.
import Darwin
import Foundation

final class FakeDaemon: @unchecked Sendable {
    let path: String
    /// method → `result` JSON text; a method without an entry gets an error.
    private var results: [String: [String]]
    private let lock = NSLock()
    private var received: [String] = []
    private var listenFD: Int32 = -1
    private var clientFD: Int32 = -1

    init(path: String, results: [String: [String]]) throws {
        self.path = path
        self.results = results
        unlink(path)
        listenFD = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: Array(path.utf8)) }
        let bound = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(listenFD, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0, listen(listenFD, 4) == 0 else { throw POSIXError(.EADDRINUSE) }
        Thread { [self] in acceptLoop() }.start()
    }

    /// Methods received so far, in order.
    var methods: [String] { lock.withLock { received } }

    func push(_ line: String) {
        let fd = lock.withLock { clientFD }
        _ = (line + "\n").withCString { write(fd, $0, strlen($0)) }
    }

    /// Closes the listener and the client, as a daemon that exits.
    func stop() {
        lock.withLock {
            shutdown(clientFD, SHUT_RDWR)
            close(clientFD)
            close(listenFD)
            clientFD = -1
            listenFD = -1
        }
        unlink(path)
    }

    private func acceptLoop() {
        while true {
            let fd = accept(lock.withLock { listenFD }, nil, nil)
            guard fd >= 0 else { return }
            lock.withLock { clientFD = fd }
            serve(fd)
        }
    }

    private func serve(_ fd: Int32) {
        var buffer = [UInt8]()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(fd, &chunk, chunk.count)
            guard n > 0 else { return }
            buffer += chunk[..<n]
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = Data(buffer[..<nl])
                buffer.removeSubrange(...nl)
                answer(line)
            }
        }
    }

    private func answer(_ line: Data) {
        guard let req = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
              let method = req["method"] as? String, let id = req["id"] as? Int
        else { return }
        let result: String? = lock.withLock {
            received.append(method)
            // Each scripted result is used once; the last one repeats.
            guard var queue = results[method], !queue.isEmpty else { return nil }
            let next = queue.count > 1 ? queue.removeFirst() : queue[0]
            results[method] = queue
            return next
        }
        if let result {
            push(#"{"jsonrpc":"2.0","id":\#(id),"result":\#(result)}"#)
        } else {
            push(#"{"jsonrpc":"2.0","id":\#(id),"error":{"code":-32601,"message":"unknown method \#(method)"}}"#)
        }
    }
}

/// Line `n` (0-based) of a fixture.
func fixtureLine(_ file: String, _ n: Int) throws -> String {
    try String(contentsOf: fixturesDir.appendingPathComponent(file), encoding: .utf8)
        .split(separator: "\n").map(String.init)[n]
}

/// The `result` of a fixture's response line, as JSON text.
func fixtureResult(_ file: String, _ n: Int) throws -> String {
    let json = try JSONSerialization.jsonObject(with: Data(fixtureLine(file, n).utf8)) as! [String: Any]
    let data = try JSONSerialization.data(withJSONObject: json["result"]!, options: .fragmentsAllowed)
    return String(decoding: data, as: UTF8.self)
}

/// Polls `condition` on the main actor every 20 ms until it holds or `seconds` pass.
@MainActor
func eventually(_ seconds: Double = 5, _ condition: () -> Bool) async -> Bool {
    let deadline = Date().addingTimeInterval(seconds)
    while Date() < deadline {
        if condition() { return true }
        try? await Task.sleep(for: .milliseconds(20))
    }
    return condition()
}

func tempSocketPath() -> String {
    "/tmp/lull-test-\(getpid())-\(UUID().uuidString.prefix(8)).sock"
}
