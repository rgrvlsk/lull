// The store against a fake daemon replaying the fixtures: snapshot, seq order, gap →
// re-subscribe, reconnect after the daemon restarts, and an outdated daemon.
import Foundation
import Testing
@testable import LullKit

@MainActor
@Suite(.serialized) struct StoreTests {
    /// hello, daemon.status and an empty history, plus the given subscribe snapshots.
    private func results(subscribe: [String], hello: String? = nil) throws -> [String: [String]] {
        [
            "hello": [try hello ?? fixtureResult("hello.ndjson", 1)],
            "subscribe": subscribe,
            "servers.history": ["[]"],
            "daemon.status": [try fixtureResult("daemon-status.ndjson", 1)],
        ]
    }

    @Test func replaysSubscribeAndReSubscribesOnAGap() async throws {
        let first = try fixtureResult("subscribe.ndjson", 1)
        let second = try fixtureResult("subscribe.ndjson", 6)
        let daemon = try FakeDaemon(path: tempSocketPath(), results: results(subscribe: [first, second]))
        defer { daemon.stop() }
        let store = Store(socketPath: daemon.path)
        let run = Task { await store.run() }
        defer { run.cancel() }

        #expect(await eventually { store.phase == .connected })
        #expect(store.servers.count == 1)
        #expect(store.status?.activityHealth == .failed)

        // seq 42 adds a server, 43 updates one; 46 skips 44–45, so the store re-subscribes.
        daemon.push(try fixtureLine("subscribe.ndjson", 2))
        daemon.push(try fixtureLine("subscribe.ndjson", 3))
        #expect(await eventually { store.servers.count == 2 })
        daemon.push(try fixtureLine("subscribe.ndjson", 4))
        #expect(await eventually { daemon.methods.filter { $0 == "subscribe" }.count == 2 })
        #expect(await eventually { store.servers.keys.sorted() == ["31002-1791600000000"] })
    }

    @Test func reconnectsAndReSubscribesAfterTheDaemonRestarts() async throws {
        let path = tempSocketPath()
        let snapshot = try fixtureResult("subscribe.ndjson", 1)
        var daemon = try FakeDaemon(path: path, results: results(subscribe: [snapshot]))
        let store = Store(socketPath: path)
        let run = Task { await store.run() }
        defer { run.cancel() }
        #expect(await eventually { store.phase == .connected })

        daemon.stop()
        #expect(await eventually { store.phase != .connected })
        #expect(store.servers.isEmpty)

        daemon = try FakeDaemon(path: path, results: results(subscribe: [snapshot]))
        defer { daemon.stop() }
        #expect(await eventually(8) { store.phase == .connected })
        #expect(daemon.methods.prefix(2) == ["hello", "subscribe"])
        #expect(store.servers.count == 1)
    }

    @Test func aDaemonOlderThanTheAppIsAMismatch() async throws {
        let old = try fixtureResult("hello.ndjson", 1).replacingOccurrences(of: #""minor":3"#, with: #""minor":2"#)
        let daemon = try FakeDaemon(path: tempSocketPath(), results: results(subscribe: [], hello: old))
        defer { daemon.stop() }
        let store = Store(socketPath: daemon.path)
        let run = Task { await store.run() }
        defer { run.cancel() }
        #expect(await eventually { store.phase == .protocolMismatch(daemonVersion: "0.1.0") })
        #expect(!daemon.methods.contains("subscribe"))
    }

    @Test func noDaemonIsNotRunning() async {
        let store = Store(socketPath: tempSocketPath())
        let run = Task { await store.run() }
        defer { run.cancel() }
        #expect(await eventually { store.phase == .daemonNotRunning })
    }

    @Test func actionsApplyTheReturnedServerAndReportErrors() async throws {
        let snapshot = try fixtureResult("subscribe.ndjson", 1)
        var scripted = try results(subscribe: [snapshot])
        scripted["servers.pin"] = [try fixtureResult("servers-pin.ndjson", 1)]
        let daemon = try FakeDaemon(path: tempSocketPath(), results: scripted)
        defer { daemon.stop() }
        let store = Store(socketPath: daemon.path)
        let run = Task { await store.run() }
        defer { run.cancel() }
        #expect(await eventually { store.phase == .connected })

        await store.pin("22249-1791530155000", true)
        #expect(store.servers["22249-1791530155000"]?.pinSource == .user)
        #expect(store.actionError == nil)
        await store.stop("22249-1791530155000")
        #expect(store.actionError == "unknown method servers.stop")
    }
}
