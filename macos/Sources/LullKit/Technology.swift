// What a server is built with, for its tile when the project has no icon of its own
// (contract §10): the daemon's framework when it names one, else an MCP server, else the
// runtime its command starts. Display only. Project files are read with the same limits as
// the project icon (`ProjectIcon.read`), off the main thread.
import Foundation

public enum Technology: String, Sendable, CaseIterable {
    case vite, next, astro, nuxt, remix, wrangler, laravel, rails, django
    case mcp, node, bun, deno, python, php, ruby, go, rust, elixir

    /// The server's technology, or nil when neither its framework nor its command says.
    public static func of(framework: Framework, command: String, mcpProject: Bool) -> Technology? {
        if let named = Self(framework) { return named }
        if mcpProject || mentionsMCP(command) { return .mcp }
        return runtime(command: command)
    }

    private init?(_ framework: Framework) {
        switch framework {
        case .vite: self = .vite
        case .next: self = .next
        case .astro: self = .astro
        case .nuxt: self = .nuxt
        case .remix: self = .remix
        case .wrangler: self = .wrangler
        case .artisan: self = .laravel
        case .rails: self = .rails
        case .django: self = .django
        case .phpBuiltin: self = .php
        case .pythonHttp: self = .python
        case .generic, .unknown: return nil
        }
    }

    // MARK: MCP

    private static let mcpWord = try! NSRegularExpression(
        pattern: #"(?:^|[^a-z0-9])(?:fast)?mcp(?:$|[^a-z0-9])|modelcontextprotocol"#, options: .caseInsensitive)

    /// Whether a command or name says MCP: `@modelcontextprotocol/inspector`, `mcp-proxy`,
    /// `fastmcp run`, `server_mcp.py`, but not `mcpe` or `xmcpd`.
    public static func mentionsMCP(_ text: String) -> Bool {
        mcpWord.firstMatch(in: text, range: NSRange(location: 0, length: (text as NSString).length)) != nil
    }

    /// Whether the project in `cwd` (or its git root) is an MCP server: its directory is
    /// named like one, or its manifest depends on an MCP SDK.
    public static func isMCPProject(cwd: String) -> Bool {
        ProjectIcon.searchRoots(cwd: cwd).contains { root in
            mentionsMCP(root.lastPathComponent) || dependsOnMCP(root)
        }
    }

    static let mcpPackages: Set = [
        "@modelcontextprotocol/sdk", "fastmcp", "mcp-framework", "mcp-handler", "@vercel/mcp-adapter",
    ]
    private static let mcpRequirement = try! NSRegularExpression(
        pattern: #"(?:^|["'\s])(?:mcp|fastmcp)\s*(?:[\[<>=~!;"']|$)"#, options: [.caseInsensitive, .anchorsMatchLines])

    static func dependsOnMCP(_ root: URL) -> Bool {
        struct Package: Decodable {
            var dependencies: [String: String]?
            var devDependencies: [String: String]?
        }
        if let data = ProjectIcon.read(root.appendingPathComponent("package.json"), root: root),
           let package = try? JSONDecoder().decode(Package.self, from: data) {
            let names = Set((package.dependencies ?? [:]).keys).union((package.devDependencies ?? [:]).keys)
            if !names.isDisjoint(with: mcpPackages) { return true }
        }
        return ["pyproject.toml", "requirements.txt"].contains { name in
            guard let data = ProjectIcon.read(root.appendingPathComponent(name), root: root) else { return false }
            let text = String(decoding: data, as: UTF8.self)
            return mcpRequirement.firstMatch(in: text, range: NSRange(location: 0, length: (text as NSString).length)) != nil
        }
    }

    // MARK: Runtime

    static let runtimes: [String: Technology] = [
        "node": .node, "npm": .node, "npx": .node, "pnpm": .node, "pnpx": .node, "yarn": .node,
        "tsx": .node, "ts-node": .node, "nodemon": .node,
        "bun": .bun, "bunx": .bun, "deno": .deno,
        "python": .python, "uv": .python, "uvx": .python, "uvicorn": .python, "gunicorn": .python,
        "hypercorn": .python, "flask": .python, "fastapi": .python, "poetry": .python, "pipenv": .python,
        "php": .php, "composer": .php, "frankenphp": .php,
        "ruby": .ruby, "bundle": .ruby, "rackup": .ruby, "puma": .ruby,
        "go": .go, "air": .go, "cargo": .rust, "mix": .elixir, "elixir": .elixir, "iex": .elixir,
    ]

    /// The runtime the command starts, from its executable: `npm run dev` is Node,
    /// `python3.12 app.py` Python, a binary under `target/` Rust. Leading `NAME=value`
    /// assignments and `env` are skipped.
    public static func runtime(command: String) -> Technology? {
        let words = command.split(separator: " ").drop { $0.contains("=") || $0 == "env" }
        guard let first = words.first else { return nil }
        if first.contains("/target/debug/") || first.contains("/target/release/") { return .rust }
        var name = (String(first) as NSString).lastPathComponent.lowercased()
        name = String(name.prefix { !$0.isNumber && $0 != "." }).trimmingCharacters(in: .init(charactersIn: "-"))
        return runtimes[name]
    }
}
