// Who started a server, as a small mark beside its port (contract §10, origin badge): a
// person for the user, the agent's own logo in its colours for a coding agent, and the
// agent's name as a badge for one lull has no logo for. The tooltip names it in full.
import SwiftUI
import LullKit

struct OriginMark: View {
    var origin: Origin

    var body: some View {
        Group {
            if origin.kind != .agent {
                Image(systemName: "person.fill")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 14, height: 14)
            } else if let agent = Self.agents[origin.agent ?? ""], let mark = Brand.mark("agent-\(origin.agent ?? "")") {
                Image(nsImage: mark)
                    .renderingMode(agent.monochrome ? .template : .original)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 14, height: 14)
                    .foregroundStyle(.primary)
            } else {
                Badge(text: Format.origin(origin), tint: .purple)
            }
        }
        .help(help)
        .accessibilityLabel(help)
    }

    /// Agents lull ships a logo for (assets/logos), by `origin.agent`. Monochrome logos are
    /// drawn in the text colour so they read on both panels.
    static let agents: [String: (name: String, monochrome: Bool)] = [
        "claude": ("Claude Code", false),
        "codex": ("Codex", false),
        "gemini": ("Gemini CLI", false),
        "cursor": ("Cursor", true),
        "copilot": ("GitHub Copilot", false),
        "opencode": ("opencode", true),
        "antigravity": ("Antigravity", false),
    ]

    private var help: String {
        guard origin.kind == .agent else { return "Started by you" }
        let name = Self.agents[origin.agent ?? ""]?.name ?? origin.agent ?? "an agent"
        let session = origin.sessionId.map { " · session \($0.prefix(8))" } ?? ""
        return "Started by \(name)\(session)"
    }
}
