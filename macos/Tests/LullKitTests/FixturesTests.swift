// Decodes every golden exchange in ../../../protocol/fixtures. In M0 only the JSON-RPC
// envelope is checked; the typed decode against LullKit's Codable models is disabled
// until WP4.1 provides them.
import Foundation
import Testing
@testable import LullKit

private let fixturesDir = URL(fileURLWithPath: #filePath)
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

    @Test(.disabled("until WP4.1 provides the Codable models"), arguments: try fixtureFiles())
    func fixtureDecodesWithTypedModels(file: URL) throws {}
}
