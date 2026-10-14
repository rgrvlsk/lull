// One stopped-server record: project, command, stop time and reason. Restart asks first,
// showing the exact command; Show log only for records that are restarting or failed.
import SwiftUI
import LullKit

struct StoppedRow: View {
    @Environment(Store.self) private var store
    @Environment(\.colorScheme) private var scheme
    var record: StoppedServer
    @State private var hovering = false
    @State private var confirming = false

    private var restartable: Bool { !record.argv.isEmpty && record.status != .restarting }
    private var hasLog: Bool { record.status == .restarting || record.status == .restartFailed }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                ProjectTile(cwd: record.cwd, framework: record.framework, command: record.command).saturation(0).opacity(0.7)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text(record.project).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                        Text(Format.ports(record.ports))
                            .font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
                    }
                    Text(record.command)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .help(Format.tilde(record.cwd))
                Spacer(minLength: 8)
                if hovering && !confirming {
                    HStack(spacing: 2) {
                        if hasLog {
                            IconButton(symbol: "doc.text", help: "Show log") { System.open(LullPaths.restartLog(record.id)) }
                        }
                        if restartable {
                            IconButton(symbol: "arrow.clockwise", help: "Restart…") { confirming = true }
                        }
                        IconButton(symbol: "xmark", help: "Forget") { Task { await store.forget(record.id) } }
                    }
                } else {
                    VStack(alignment: .trailing, spacing: 2) {
                        statusLine
                        TimelineView(.periodic(from: .now, by: 30)) { context in
                            Text(Format.ago(record.stoppedAt, now: context.date))
                                .font(.system(size: 10)).foregroundStyle(.tertiary)
                        }
                    }
                }
            }
            if confirming { confirmation }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .contentShape(Rectangle())
        .background(hovering ? Color.primary.opacity(0.05) : .clear)
        .onHover { hovering = $0 }
        .contextMenu { menu }
    }

    @ViewBuilder private var statusLine: some View {
        switch record.status {
        case .restarting:
            Text("restarting…").font(.system(size: 11, weight: .medium)).foregroundStyle(StatusColor.soon(scheme))
        case .restartFailed:
            Text("restart failed").font(.system(size: 11, weight: .medium)).foregroundStyle(StatusColor.problem(scheme))
        default:
            Text(Format.reason(record.reason)).font(.system(size: 11)).foregroundStyle(.secondary)
        }
    }

    /// Inline, so the popover never loses focus to a modal alert.
    private var confirmation: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Restart runs, in \(Format.tilde(record.cwd)):")
                .font(.system(size: 11)).foregroundStyle(.secondary)
            Text(Format.restartCommand(record))
                .font(.system(size: 11, design: .monospaced))
                .textSelection(.enabled)
                .padding(6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.06)))
            HStack {
                Spacer()
                Button("Cancel") { confirming = false }
                    .keyboardShortcut(.cancelAction)
                Button("Restart") {
                    confirming = false
                    Task { await store.restart(record.id) }
                }
                .keyboardShortcut(.defaultAction)
            }
            .controlSize(.small)
        }
        .padding(.leading, 34)
    }

    @ViewBuilder private var menu: some View {
        if restartable { Button("Restart…") { confirming = true } }
        if hasLog { Button("Show log") { System.open(LullPaths.restartLog(record.id)) } }
        Button("Copy command") { System.copy(record.command) }
        Button("Reveal in Finder") { System.reveal(record.cwd) }
        Button("Open Terminal here") { System.openTerminal(record.cwd) }
        Divider()
        Button("Forget") { Task { await store.forget(record.id) } }
    }
}
