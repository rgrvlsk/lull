// Preview data: the fixtures' servers plus worst cases (long paths, 30 servers, none, a
// four-digit RSS, a five-port server). Debug builds also use it when launched with
// `LULL_PREVIEW=fixtures|worst|empty|not-running|mismatch`, so the panel can be checked
// without a daemon.
#if DEBUG
import Foundation
import SwiftUI
import LullKit

@MainActor
enum PreviewData {
    private static let template = #"""
    {"id":"22249-1791530155000","pgid":22249,"pids":[22250,22276,22293],"command":"npm run dev",
     "argv":["npm","run","dev"],"cwd":"/Users/x/proj/apps/crm","project":"crm","framework":"vite",
     "ports":[5187],"primaryUrl":"http://127.0.0.1:5187/","origin":{"kind":"agent","agent":"claude"},
     "startedAt":"2026-10-09T00:35:55Z","cpuPercent":0.1,"rssBytes":182452224,
     "lastActivityAt":"2026-10-11T20:12:00Z","lastActivitySource":"tab","idleMinutes":30,
     "autoStop":true,"pinned":false,"pinSource":null,"state":"running",
     "stopsAt":"2026-10-11T20:42:00Z","tabs":[]}
    """#
    private static let recordTemplate = #"""
    {"id":"18100-1791400000000","project":"shop","framework":"next","command":"next dev",
     "argv":["next","dev"],"cwd":"/Users/x/proj/shop","ports":[3000],"origin":{"kind":"agent","agent":"codex"},
     "stoppedAt":"2026-10-11T18:30:00Z","reason":"idle","restartEnv":{"PORT":"3000"},"tabs":[],"status":"stopped"}
    """#

    static func server(
        _ id: String, project: String, framework: Framework = .vite, ports: [Int] = [5173],
        command: String = "npm run dev", cwd: String? = nil, agent: String? = "claude",
        state: ServerState = .running, stopsIn: TimeInterval? = 25 * 60, pinSource: PinSource? = nil,
        rss: Int64 = 182_452_224, cpu: Double = 0.4
    ) -> Server {
        var s = try! JSONDecoder.lull().decode(Server.self, from: Data(template.utf8))
        s.id = id
        s.project = project
        s.framework = framework
        s.ports = ports
        s.primaryUrl = ports.first.map { "http://127.0.0.1:\($0)/" }
        s.command = command
        s.cwd = cwd ?? "/Users/x/proj/\(project)"
        s.origin = agent.map { Origin(kind: .agent, agent: $0) } ?? Origin(kind: .user)
        s.state = state
        s.stopsAt = stopsIn.map { Date.now.addingTimeInterval($0) }
        s.pinned = pinSource != nil
        s.pinSource = pinSource
        s.rssBytes = rss
        s.cpuPercent = cpu
        return s
    }

    static func record(
        _ id: String, project: String, framework: Framework = .next, argv: [String] = ["next", "dev"],
        reason: StopReason = .idle, status: RestartStatus = .stopped, ago: TimeInterval = 40 * 60
    ) -> StoppedServer {
        var r = try! JSONDecoder.lull().decode(StoppedServer.self, from: Data(recordTemplate.utf8))
        r.id = id
        r.project = project
        r.framework = framework
        r.argv = argv
        r.command = argv.isEmpty ? "node server.js --port 3000" : argv.joined(separator: " ")
        r.reason = reason
        r.status = status
        r.stoppedAt = Date.now.addingTimeInterval(-ago)
        r.cwd = "/Users/x/proj/\(project)"
        return r
    }

    static var fixtures: Store {
        .preview(
            servers: [
                server("1-1", project: "crm", ports: [5187, 56306]),
                server("2-1", project: "crm", framework: .wrangler, ports: [8787], command: "wrangler dev",
                       state: .stoppingSoon, stopsIn: 95),
                server("3-1", project: "site", framework: .astro, ports: [4321], command: "astro dev",
                       agent: nil, stopsIn: nil),
                server("4-1", project: "api", framework: .pythonHttp, ports: [8000],
                       command: "python -m http.server 8000", stopsIn: nil, pinSource: .env),
            ],
            history: [
                record("h1", project: "shop"),
                record("h2", project: "docs", framework: .vite, argv: [], reason: .exited, ago: 3 * 3600),
                record("h3", project: "blog", framework: .astro, argv: ["astro", "dev"], reason: .manual,
                       status: .restartFailed, ago: 90),
            ])
    }

    static var worst: Store {
        let long = "/Users/x/Documents/clients/a-very-long-client-name/monorepo/packages/applications/frontend-dashboard-experimental"
        var servers = (0..<26).map { i in
            server("w\(i)", project: "project-\(i % 9)", framework: [.vite, .next, .astro, .generic][i % 4],
                   ports: [3000 + i], stopsIn: Double(30 + i * 61))
        }
        servers.append(server("big", project: "a-project-name-that-goes-on-and-on-and-on-and-on", ports: [5173],
                              command: "node --max-old-space-size=8192 ./node_modules/.bin/vite --host 127.0.0.1 --port 5173 --strictPort",
                              cwd: long, rss: 4_831_838_208, cpu: 187.3))
        servers.append(server("ports", project: "workerd", framework: .wrangler, ports: [8787, 9229, 9230, 9231, 9232],
                              command: "wrangler dev --inspector-port 9229", agent: "a-custom-agent-with-a-long-name"))
        servers.append(server("stop", project: "project-1", state: .stopping, stopsIn: 0))
        servers.append(server("nolisten", project: "project-2", ports: [], state: .noListener, stopsIn: nil))
        return .preview(servers: servers, history: (0..<20).map {
            record("r\($0)", project: "stopped-\($0)", ago: Double($0) * 1800)
        })
    }

    /// One row per technology tile and origin mark.
    static var marks: Store {
        let commands: [(Framework, String)] = [
            (.vite, "vite"), (.next, "next dev"), (.astro, "astro dev"), (.nuxt, "nuxi dev"),
            (.remix, "remix vite:dev"), (.wrangler, "wrangler dev"), (.artisan, "php artisan serve"),
            (.rails, "bin/rails server"), (.django, "python manage.py runserver"),
            (.generic, "npx @modelcontextprotocol/inspector"), (.generic, "node server.js"),
            (.generic, "bun --hot index.ts"), (.generic, "deno task dev"), (.generic, "uvicorn main:app"),
            (.phpBuiltin, "php -S 127.0.0.1:8080"), (.generic, "bundle exec puma"), (.generic, "go run ."),
            (.generic, "cargo run"), (.generic, "mix phx.server"), (.generic, "./bin/example-app"),
        ]
        let agents: [String?] = [nil, "claude", "codex", "gemini", "cursor", "copilot", "opencode",
                                 "antigravity", "augment"]
        return .preview(servers: commands.enumerated().map { i, entry in
            server("m\(i)", project: "project-\(i / 4)", framework: entry.0, ports: [3000 + i],
                   command: entry.1, agent: agents[i % agents.count])
        })
    }

    static var empty: Store { .preview(servers: []) }

    /// The store for `LULL_PREVIEW`, or `nil` to connect to the daemon.
    static func launchStore() -> Store? {
        switch ProcessInfo.processInfo.environment["LULL_PREVIEW"] {
        case "fixtures": fixtures
        case "worst": worst
        case "marks": marks
        case "empty": empty
        case "not-running": .preview(phase: .daemonNotRunning, servers: [])
        case "mismatch": .preview(phase: .protocolMismatch(daemonVersion: "0.2.0"), servers: [])
        default: nil
        }
    }
}

#Preview("Fixtures") { MenuView(openSettings: {}).environment(PreviewData.fixtures) }
#Preview("Worst case") { MenuView(openSettings: {}).environment(PreviewData.worst) }
#Preview("No servers") { MenuView(openSettings: {}).environment(PreviewData.empty) }
#Preview("Not running") {
    MenuView(openSettings: {}).environment(Store.preview(phase: .daemonNotRunning, servers: []))
}
#endif
