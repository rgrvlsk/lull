// Decodes every golden exchange in ../../../protocol/fixtures: the JSON-RPC envelope, then
// every result and notification the app uses with LullKit's Codable models.
import Foundation
import Testing
@testable import LullKit

let fixturesDir = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()  // LullKitTests
    .deletingLastPathComponent()  // Tests
    .deletingLastPathComponent()  // macos
    .deletingLastPathComponent()  // repo root
    .appendingPathComponent("protocol/fixtures")

private func fixtureFiles() throws -> [URL] {
    try FileManager.default.contentsOfDirectory(at: fixturesDir, includingPropertiesForKeys: nil)
        .filter { $0.pathExtension == "ndjson" }
        .sorted { $0.lastPathComponent < $1.lastPathComponent }
}

@Suite struct FixturesTests {
    @Test func fixtureSetIsPresent() throws {
        #expect(try fixtureFiles().count >= 23)
    }

    @Test func protocolVersionMatchesHelloFixture() throws {
        let line = try String(contentsOf: fixturesDir.appendingPathComponent("hello.ndjson"), encoding: .utf8)
            .split(separator: "\n")[1]
        struct Result: Decodable { let protocol_: ProtocolVersion
            enum CodingKeys: String, CodingKey { case protocol_ = "protocol" } }
        struct Response: Decodable { let result: Result }
        let response = try JSONDecoder().decode(Response.self, from: Data(line.utf8))
        #expect(response.result.protocol_ == LullKit.protocolVersion)
    }

    @Test(arguments: try fixtureFiles())
    func fixtureIsAValidJSONRPCExchange(file: URL) throws {
        let text = try String(contentsOf: file, encoding: .utf8)
        var pending = Set<String>()
        for (i, line) in text.split(separator: "\n").enumerated() {
            let where_ = "\(file.lastPathComponent):\(i + 1)"
            let object = try JSONSerialization.jsonObject(with: Data(line.utf8))
            let msg = try #require(object as? [String: Any], "\(where_) is not an object")
            #expect(msg["jsonrpc"] as? String == "2.0", "\(where_)")
            let id = msg["id"].map { "\($0)" }
            if let method = msg["method"] as? String {
                if let id { pending.insert(id) } else {
                    #expect(method.hasPrefix("server."), "\(where_)")
                }
            } else {
                let id = try #require(id, "\(where_) response without id")
                #expect(pending.remove(id) != nil, "\(where_) response to unknown id")
                #expect((msg["result"] != nil) != (msg["error"] != nil), "\(where_)")
            }
        }
        #expect(pending.isEmpty, "\(file.lastPathComponent): unanswered requests")
    }

    @Test(arguments: try fixtureFiles())
    func fixtureDecodesWithTypedModels(file: URL) throws {
        let decoder = JSONDecoder.lull()
        var pending: [String: String] = [:]
        var decoded = 0
        for (i, line) in try String(contentsOf: file, encoding: .utf8).split(separator: "\n").enumerated() {
            let where_ = "\(file.lastPathComponent):\(i + 1)"
            let data = Data(line.utf8)
            let msg = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
            let id = msg["id"].map { "\($0)" }
            if let method = msg["method"] as? String {
                if let id { pending[id] = method } else {
                    _ = try decoder.decode(DaemonNotification.self, from: data)
                    decoded += 1
                }
                continue
            }
            let responseID = try #require(id, "\(where_)")
            let method = try #require(pending[responseID], "\(where_)")
            pending[responseID] = nil
            if let error = msg["error"] as? [String: Any] {
                if error["code"] as? Int == -32010 {
                    let data = try #require(error["data"])
                    let body = try JSONSerialization.data(withJSONObject: data)
                    let mismatch = try decoder.decode(ProtocolMismatch.self, from: body)
                    #expect(mismatch.protocol == LullKit.protocolVersion, "\(where_)")
                }
                continue
            }
            let raw = try #require(msg["result"])
            let result = try JSONSerialization.data(withJSONObject: raw, options: .fragmentsAllowed)
            switch method {
            case "hello": _ = try decoder.decode(HelloResult.self, from: result)
            case "servers.list": _ = try decoder.decode([Server].self, from: result)
            case "servers.pin", "servers.setAutoStop", "servers.touch": _ = try decoder.decode(Server.self, from: result)
            case "servers.history": _ = try decoder.decode([StoppedServer].self, from: result)
            case "servers.restart": _ = try decoder.decode(RestartResult.self, from: result)
            case "servers.open": _ = try decoder.decode(OpenResult.self, from: result)
            case "config.get", "config.set": _ = try decoder.decode(Config.self, from: result)
            case "subscribe": _ = try decoder.decode(SubscribeResult.self, from: result)
            case "daemon.status": _ = try decoder.decode(DaemonStatus.self, from: result)
            case "servers.stop", "servers.forget": _ = try decoder.decode(Empty.self, from: result)
            default: continue  // browser.* and tab.* are not the app's
            }
            decoded += 1
        }
        let browserOnly = ["browser-", "tab-", "hello-browser", "hello-protocol-mismatch"]
        if !browserOnly.contains(where: file.lastPathComponent.hasPrefix) {
            #expect(decoded > 0, "\(file.lastPathComponent) decoded nothing")
        }
    }

    @Test func newerDaemonValuesFallBackAndUnknownFieldsAreIgnored() throws {
        let line = try String(contentsOf: fixturesDir.appendingPathComponent("servers-list.ndjson"), encoding: .utf8)
            .split(separator: "\n")[1]
        var json = try #require(try JSONSerialization.jsonObject(with: Data(line.utf8)) as? [String: Any])
        var servers = try #require(json["result"] as? [[String: Any]])
        servers[0]["framework"] = "deno-fresh"
        servers[0]["state"] = "hibernating"
        servers[0]["somethingNew"] = ["a": 1]
        json["result"] = servers
        let body = try JSONSerialization.data(withJSONObject: json["result"]!)
        let decoded = try JSONDecoder.lull().decode([Server].self, from: body)
        #expect(decoded[0].framework == .unknown)
        #expect(decoded[0].state == .unknown)
        #expect(decoded[0].pinSource == nil)
        #expect(decoded[1].restartedFrom?.origin.agent == "claude")
    }
}
