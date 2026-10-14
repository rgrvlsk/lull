// One live server: port, origin mark and pin badge, countdown, CPU/RSS; Open, Stop and
// Pin/Unpin on hover, the rest in the context menu (contract §10).
import SwiftUI
import LullKit

struct ServerRow: View {
    @Environment(Store.self) private var store
    @Environment(\.colorScheme) private var scheme
    var server: Server
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 10) {
            ProjectTile(cwd: server.cwd, framework: server.framework, command: server.command)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(Format.ports(server.ports))
                        .font(.system(size: 13, weight: .semibold, design: .monospaced))
                    OriginMark(origin: server.origin)
                    if server.pinned {
                        Badge(text: server.pinSource == .env ? "pinned (env)" : "pinned", tint: .teal)
                    }
                }
                Text(server.command)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            .help(Format.tilde(server.cwd))
            Spacer(minLength: 8)
            if hovering {
                actions
            } else {
                VStack(alignment: .trailing, spacing: 2) {
                    status
                    Text("\(server.cpuPercent, specifier: "%.1f") % · \(Format.bytes(server.rssBytes))")
                        .font(.system(size: 10).monospacedDigit())
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .contentShape(Rectangle())
        .background(hovering ? Color.primary.opacity(0.05) : .clear)
        .onHover { hovering = $0 }
        .contextMenu { menu }
    }

    @ViewBuilder private var status: some View {
        switch server.state {
        case .stopping:
            Text("stopping…").font(.system(size: 11, weight: .medium))
                .foregroundStyle(StatusColor.problem(scheme))
        case .noListener:
            Text("no listener").font(.system(size: 11)).foregroundStyle(.secondary)
        default:
            if let stopsAt = server.stopsAt {
                let soon = server.state == .stoppingSoon
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text("stops in \(Format.countdown(to: stopsAt, now: context.date))")
                        .font(.system(size: 11, weight: soon ? .semibold : .regular).monospacedDigit())
                        .foregroundStyle(soon ? StatusColor.soon(scheme) : .secondary)
                }
            } else {
                Text(server.pinned ? "kept running" : "no auto-stop")
                    .font(.system(size: 11)).foregroundStyle(.secondary)
            }
        }
    }

    private var actions: some View {
        HStack(spacing: 2) {
            if server.state == .stoppingSoon {
                IconButton(symbol: "hand.raised", help: "Keep alive") { Task { await store.touch(server.id) } }
            }
            IconButton(symbol: "safari", help: "Open") { Task { await store.open(server.id) } }
                .disabled(server.primaryUrl == nil)
            IconButton(symbol: server.pinned ? "pin.slash" : "pin", help: server.pinned ? "Unpin" : "Pin") {
                Task { await store.pin(server.id, !server.pinned) }
            }
            IconButton(symbol: "stop.circle", help: "Stop", tint: StatusColor.problem(scheme)) {
                Task { await store.stop(server.id) }
            }
            .disabled(server.state == .stopping)
        }
    }

    @ViewBuilder private var menu: some View {
        Button("Open") { Task { await store.open(server.id) } }.disabled(server.primaryUrl == nil)
        Button(server.pinned ? "Unpin" : "Pin") { Task { await store.pin(server.id, !server.pinned) } }
        Button("Keep alive") { Task { await store.touch(server.id) } }
        Menu("Auto-stop") {
            Button("Follow policy") { Task { await store.setAutoStop(server.id, nil) } }
            Button("Always") { Task { await store.setAutoStop(server.id, true) } }
            Button("Never") { Task { await store.setAutoStop(server.id, false) } }
        }
        Divider()
        Button("Copy URL") { System.copy(server.primaryUrl ?? "") }.disabled(server.primaryUrl == nil)
        Button("Copy command") { System.copy(server.command) }
        Button("Reveal in Finder") { System.reveal(server.cwd) }
        Divider()
        Button("Stop") { Task { await store.stop(server.id) } }.disabled(server.state == .stopping)
    }
}

struct IconButton: View {
    var symbol: String
    var help: String
    var tint: Color = .secondary
    var action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .medium))
                .frame(width: 26, height: 24)
                .foregroundStyle(tint)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(hovering ? 0.1 : 0)))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help(help)
        .accessibilityLabel(help)
    }
}
