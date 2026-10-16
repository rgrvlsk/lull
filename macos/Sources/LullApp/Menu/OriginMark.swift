// Who started a server, as a small mark beside its port (contract §10, origin badge): a
// person for the user, the agent's own logo in its colours for a coding agent, and a
// generic agent glyph for one lull has no logo for. Its hover card names the agent in
// full, with the session id and a copy button (§11) and when the server started.
import SwiftUI
import LullKit

struct OriginMark: View {
    var origin: Origin
    var startedAt: Date

    var body: some View {
        mark(size: 14)
            .hoverCard { card }
            .accessibilityLabel(title)
    }

    @ViewBuilder private func mark(size: CGFloat) -> some View {
        if origin.kind != .agent {
            Image(systemName: "person.fill")
                .font(.system(size: size * 0.7, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: size, height: size)
        } else if let agent = Self.agents[origin.agent ?? ""], let logo = Brand.mark("agent-\(origin.agent ?? "")") {
            Image(nsImage: logo)
                .renderingMode(agent.monochrome ? .template : .original)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: size, height: size)
                .foregroundStyle(.primary)
        } else {
            Image(systemName: "sparkles")
                .font(.system(size: size * 0.75, weight: .semibold))
                .foregroundStyle(.purple)
                .frame(width: size, height: size)
        }
    }

    private var card: some View {
        VStack(alignment: .leading, spacing: 8) {
            CardTitle(title: title, subtitle: origin.kind == .agent ? "Coding agent" : "Started outside an agent") {
                mark(size: 24)
            }
            VStack(alignment: .leading, spacing: 4) {
                if let session = origin.sessionId {
                    CardField(label: "Session", value: session, copy: true)
                }
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    CardField(label: "Started", value: Format.ago(startedAt, now: context.date))
                }
            }
        }
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

    private var title: String {
        guard origin.kind == .agent else { return "Started by you" }
        return "Started by " + (Self.agents[origin.agent ?? ""]?.name ?? origin.agent ?? "an agent")
    }
}
