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
        detect(framework: framework, command: command, mcpEvidence: mcpProject ? "" : nil)?.technology
    }

    /// The server's technology and, for display, how it was told.
    public static func detect(
        framework: Framework, command: String, mcpEvidence: String?
    ) -> (technology: Technology, reason: String)? {
        if let named = Self(framework) { return (named, "named by its command") }
        if mentionsMCP(command) { return (.mcp, "named by its command") }
        if let mcpEvidence { return (.mcp, mcpEvidence) }
        guard let runtime = runtime(command: command) else { return nil }
        let program = command.split(separator: " ").first { !$0.contains("=") && $0 != "env" } ?? ""
        return (runtime, "started with \((String(program) as NSString).lastPathComponent)")
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

    /// Whether the project in `cwd` (or its git root) is an MCP server.
    public static func isMCPProject(cwd: String) -> Bool { mcpEvidence(cwd: cwd) != nil }

    /// Why the project in `cwd` (or its git root) is an MCP server, for display: its
    /// directory is named like one, or its manifest depends on an MCP SDK. Nil when neither.
    public static func mcpEvidence(cwd: String) -> String? {
        for root in ProjectIcon.searchRoots(cwd: cwd) {
            if mentionsMCP(root.lastPathComponent) { return "its folder is named \(root.lastPathComponent)" }
            if let dependency = mcpDependency(root) { return dependency }
        }
        return nil
    }

    static let mcpPackages = [
        "@modelcontextprotocol/sdk", "fastmcp", "mcp-framework", "mcp-handler", "@vercel/mcp-adapter",
    ]
    private static let mcpRequirement = try! NSRegularExpression(
        pattern: #"(?:^|["'\s])(mcp|fastmcp)\s*(?:[\[<>=~!;"']|$)"#, options: [.caseInsensitive, .anchorsMatchLines])

    static func mcpDependency(_ root: URL) -> String? {
        struct Package: Decodable {
            var dependencies: [String: String]?
            var devDependencies: [String: String]?
        }
        if let data = ProjectIcon.read(root.appendingPathComponent("package.json"), root: root),
           let package = try? JSONDecoder().decode(Package.self, from: data) {
            let names = Set((package.dependencies ?? [:]).keys).union((package.devDependencies ?? [:]).keys)
            if let sdk = mcpPackages.first(where: names.contains) { return "package.json depends on \(sdk)" }
        }
        for name in ["pyproject.toml", "requirements.txt"] {
            guard let data = ProjectIcon.read(root.appendingPathComponent(name), root: root) else { continue }
            let text = String(decoding: data, as: UTF8.self) as NSString
            if let match = mcpRequirement.firstMatch(in: text as String, range: NSRange(location: 0, length: text.length)) {
                return "\(name) lists \(text.substring(with: match.range(at: 1)))"
            }
        }
        return nil
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
