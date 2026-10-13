// The panel: header with a status pill; then, scrolling, banners for a missing or mismatched
// daemon and dry run, live servers grouped by project and Recently stopped; a Settings · Quit
// footer.
import SwiftUI
import LullKit

struct MenuView: View {
    @Environment(Store.self) private var store
    @Environment(\.colorScheme) private var scheme
    var openSettings: () -> Void

    var body: some View {
        // The list reaches up behind the band's bottom edge, below the buildings, and a
        // copy of the band fades over it there, so scrolled rows dissolve into the skyline;
        // at rest the list starts below the band. Masking the list instead would flatten
        // its vibrant text.
        VStack(spacing: 0) {
            PanelScroll(maxHeight: Theme.maxListHeight + Self.fade, gutter: Theme.inset, fade: Self.fade) {
                VStack(alignment: .leading, spacing: 12) {
                    banners
                    list
                }
                .padding(.horizontal, Theme.inset)
            }
            footer.padding(.horizontal, Theme.inset).padding(.top, 12)
        }
        .padding(.top, PanelBackdrop.height - Self.fade)
        .padding(.bottom, Theme.inset)
        .background(alignment: .top) { PanelBackdrop() }
        .overlay(alignment: .top) {
            PanelBackdrop()
                .mask {
                    LinearGradient(stops: [
                        .init(color: .black, location: 1 - Self.fade / PanelBackdrop.height),
                        .init(color: .clear, location: 1),
                    ], startPoint: .top, endPoint: .bottom)
                }
                .allowsHitTesting(false)
        }
        .overlay(alignment: .top) { PanelHeader { pill } }
        .frame(width: Theme.panelWidth)
        .background(Color(nsColor: Brand.panel))
    }

    // MARK: Header

    /// The bottom of the band, below the buildings, where scrolled rows fade out.
    private static let fade: CGFloat = 14

    private var pill: some View {
        let (text, color) = pillState
        return HStack(spacing: 5) {
            Circle().fill(color).frame(width: 6, height: 6)
            Text(text).font(.system(size: 11, weight: .medium))
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 2.5)
        .foregroundStyle(color)
        // Over the panel colour first, so the pill reads on the bright part of the sky.
        .background(Capsule().fill(color.opacity(0.14)))
        .background(Capsule().fill(Color(nsColor: Brand.panel).opacity(0.85)))
    }

    private var pillState: (String, Color) {
        switch store.phase {
        case .connecting: return ("connecting…", .secondary)
        case .daemonNotRunning: return ("daemon not running", StatusColor.problem(scheme))
        case .protocolMismatch: return ("update lull", StatusColor.soon(scheme))
        case .connected:
            let live = store.servers.values
            let soon = live.filter { $0.state == .stoppingSoon }.count
            if live.isEmpty { return ("no servers running", .secondary) }
            let running = "\(live.count) running"
            return soon > 0
                ? ("\(running) · \(soon) stopping soon", StatusColor.soon(scheme))
                : (running, StatusColor.running(scheme))
        }
    }

    // MARK: Banners

    @ViewBuilder private var banners: some View {
        switch store.phase {
        case .daemonNotRunning:
            Banner(symbol: "bolt.slash", tint: StatusColor.problem(scheme),
                   title: "lull isn't running", text: "Start the daemon, then this panel connects by itself.",
                   command: "brew services start lull")
        case .protocolMismatch(let version):
            Banner(symbol: "arrow.down.circle", tint: StatusColor.soon(scheme),
                   title: "Update lull", text: "The running daemon (\(version)) doesn't match this app.",
                   command: "brew upgrade lull")
        default:
            EmptyView()
        }
        if store.status?.dryRun == true {
            Banner(symbol: "eye", tint: .blue, title: "Dry run",
                   text: "lull decides and logs every stop but sends no signals.")
        }
        if let error = store.status?.configError {
            Banner(symbol: "exclamationmark.triangle", tint: StatusColor.soon(scheme),
                   title: "config.toml rejected", text: "lull keeps the last good config. \(error)")
        }
        if let error = store.actionError {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Image(systemName: "exclamationmark.circle.fill")
                Text(error).frame(maxWidth: .infinity, alignment: .leading)
                Button { store.actionError = nil } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain)
                    .help("Dismiss")
            }
            .font(.system(size: 11))
            .foregroundStyle(StatusColor.problem(scheme))
            .padding(.horizontal, 4)
        }
    }

    // MARK: List

    private var groups: [(project: String, servers: [Server])] {
        Dictionary(grouping: store.sortedServers, by: \.project)
            .map { ($0.key, $0.value) }
            .sorted { $0.project.localizedStandardCompare($1.project) == .orderedAscending }
    }

    @ViewBuilder private var list: some View {
        VStack(alignment: .leading, spacing: 12) {
            if store.phase == .connected {
                if store.servers.isEmpty {
                    empty
                } else {
                    SectionTitle(text: "Live", trailing: "\(store.servers.count)")
                    ForEach(groups, id: \.project) { group in
                        Card {
                            Text(group.project)
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                                .padding(.top, 8)
                                .padding(.bottom, 2)
                            ForEach(Array(group.servers.enumerated()), id: \.element.id) { i, server in
                                if i > 0 { Divider().padding(.leading, 44) }
                                ServerRow(server: server)
                            }
                        }
                    }
                }
                if !store.history.isEmpty {
                    SectionTitle(text: "Recently stopped", trailing: "\(store.history.count)")
                        .padding(.top, 4)
                    Card {
                        ForEach(Array(store.history.enumerated()), id: \.element.id) { i, record in
                            if i > 0 { Divider().padding(.leading, 44) }
                            StoppedRow(record: record)
                        }
                    }
                }
            }
        }
        .padding(.bottom, 2)
    }

    private var empty: some View {
        VStack(spacing: 6) {
            Image(systemName: "checkmark.circle")
                .font(.system(size: 30, weight: .light))
                .foregroundStyle(.tertiary)
            Text("No dev servers running").font(.system(size: 13, weight: .medium))
            Text("Servers you or your agents start show up here.")
                .font(.system(size: 11)).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 18)
    }

    // MARK: Footer

    private var footer: some View {
        HStack {
            FooterButton(symbol: "gearshape", title: "Settings", action: openSettings)
            Spacer()
            FooterButton(symbol: "power", title: "Quit") { NSApp.terminate(nil) }
        }
    }
}

private struct Banner: View {
    var symbol: String
    var tint: Color
    var title: String
    var text: String
    var command: String?

    var body: some View {
        Card {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: symbol)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(tint)
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 4) {
                    Text(title).font(.system(size: 12, weight: .semibold))
                    Text(text).font(.system(size: 11)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    if let command {
                        HStack {
                            Text(command).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                            Spacer()
                            IconButton(symbol: "doc.on.doc", help: "Copy") { System.copy(command) }
                        }
                        .padding(.leading, 6)
                        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.06)))
                    }
                }
            }
            .padding(10)
        }
    }
}

private struct FooterButton: View {
    @Environment(\.colorScheme) private var scheme
    var symbol: String
    var title: String
    var action: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.system(size: 12, weight: .medium))
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .foregroundStyle(hovering ? AnyShapeStyle(.primary) : PanelText.muted(scheme))
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(hovering ? 0.08 : 0)))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
