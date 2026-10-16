// A server's technology from its framework, command and made-up project files.
import Foundation
import Testing
@testable import LullKit

@Suite final class TechnologyTests {
    let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("lull-tech-\(UUID().uuidString)/example-app")
            .resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    deinit { try? FileManager.default.removeItem(at: root.deletingLastPathComponent()) }

    private func write(_ name: String, _ text: String) throws {
        try Data(text.utf8).write(to: root.appendingPathComponent(name))
    }

    @Test func aNamedFrameworkWins() {
        #expect(Technology.of(framework: .vite, command: "npx mcp-proxy", mcpProject: true) == .vite)
        #expect(Technology.of(framework: .artisan, command: "php artisan serve", mcpProject: false) == .laravel)
        #expect(Technology.of(framework: .pythonHttp, command: "python3 -m http.server", mcpProject: false) == .python)
    }

    @Test func mcpServersAreNamedFromTheirCommand() {
        for command in ["npx @modelcontextprotocol/inspector", "uvx mcp-server-fetch --port 8000",
                        "fastmcp run server.py", "python server_mcp.py"] {
            #expect(Technology.of(framework: .generic, command: command, mcpProject: false) == .mcp, "\(command)")
        }
        #expect(!Technology.mentionsMCP("node mcpe-server.js"))
        #expect(!Technology.mentionsMCP("/usr/sbin/xmcpd"))
    }

    @Test func mcpProjectsAreNamedFromTheirFiles() throws {
        #expect(!Technology.isMCPProject(cwd: root.path))
        try write("package.json", #"{"name":"example-app","dependencies":{"@modelcontextprotocol/sdk":"^1.0.0"}}"#)
        #expect(Technology.isMCPProject(cwd: root.path))
        try write("package.json", #"{"name":"example-app","dependencies":{"express":"^5.0.0"}}"#)
        try write("pyproject.toml", """
            [project]
            dependencies = ["httpx>=0.27", "mcp[cli]>=1.2"]
            """)
        #expect(Technology.isMCPProject(cwd: root.path))
        try write("pyproject.toml", #"dependencies = ["mcpe>=1"]"#)
        #expect(!Technology.isMCPProject(cwd: root.path))
    }

    @Test func otherwiseTheRuntime() {
        let cases: [(String, Technology?)] = [
            ("npm run dev", .node), ("/opt/homebrew/bin/node server.js", .node), ("bun --hot index.ts", .bun),
            ("PORT=4000 python3.12 app.py", .python), ("env DEBUG=1 uvicorn main:app", .python),
            ("go run .", .go), ("./target/debug/example-app", .rust), ("mix phx.server", .elixir),
            ("./bin/example-app", nil),
        ]
        for (command, expected) in cases {
            #expect(Technology.of(framework: .generic, command: command, mcpProject: false) == expected, "\(command)")
        }
    }
}
