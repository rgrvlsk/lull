// Project icons for the rows, and whether each project is an MCP server, looked up off
// the main thread and cached by directory. Rows show the technology tile until (and
// unless) their directory has an icon.
import AppKit
import SwiftUI
import LullKit

@MainActor
@Observable
final class ProjectIcons {
    struct Icon {
        var image: CGImage
        /// Drawn at its own size on a tile of `tint`, not scaled up.
        var small: Bool
        var tint: CGColor?
        /// The file it was read from.
        var source: URL
    }

    /// Decoded icons by server cwd; a cwd without one is absent.
    private(set) var icons: [String: Icon] = [:]
    /// Why each server cwd's project is an MCP server (`Technology.mcpEvidence`); absent
    /// when it is not one.
    private(set) var mcpEvidence: [String: String] = [:]
    @ObservationIgnored private var lookups: [String: ProjectIcon.Lookup] = [:]
    @ObservationIgnored private var pending: Set<String> = []

    /// The rows' @2x pixel size.
    private static let pixels = Int(TechTile.size * 2)

    /// Looks up the icon for `cwd` once; later calls are free.
    func load(_ cwd: String) async {
        guard lookups[cwd] == nil, pending.insert(cwd).inserted else { return }
        let pixels = Self.pixels
        let (lookup, mcp) = await Task.detached(priority: .utility) {
            (ProjectIcon.lookup(cwd: cwd, pixels: pixels), Technology.mcpEvidence(cwd: cwd))
        }.value
        pending.remove(cwd)
        store(lookup, for: cwd)
        store(mcp: mcp, for: cwd)
    }

    // ponytail: re-checked only when the panel opens (stat-only mtimes), so an icon changed
    // while the panel is open shows on the next open; upgrade trigger: users expect icons to
    // update live → watch the search roots with FSEvents.
    /// Looks up again every directory whose files changed since its last lookup, and
    /// re-reads its manifests for MCP.
    func revalidate() {
        let pixels = Self.pixels
        for (cwd, old) in lookups where !pending.contains(cwd) {
            pending.insert(cwd)
            Task {
                let (fresh, mcp) = await Task.detached(priority: .utility) {
                    (ProjectIcon.isStale(old, cwd: cwd) ? ProjectIcon.lookup(cwd: cwd, pixels: pixels) : nil,
                     Technology.mcpEvidence(cwd: cwd))
                }.value
                pending.remove(cwd)
                if let fresh { store(fresh, for: cwd) }
                store(mcp: mcp, for: cwd)
            }
        }
    }

    private func store(mcp: String?, for cwd: String) {
        if mcpEvidence[cwd] != mcp { mcpEvidence[cwd] = mcp }
    }

    private func store(_ lookup: ProjectIcon.Lookup, for cwd: String) {
        lookups[cwd] = lookup
        guard icons[cwd]?.image !== lookup.image else { return }
        icons[cwd] = lookup.image.flatMap { image in
            lookup.found.map { Icon(image: image, small: $0.small, tint: lookup.tint, source: $0.url) }
        }
    }
}

/// The project's icon on the row tile. A small icon keeps its own pixel size, crisp, on a
/// tile tinted with its colour. The technology tile when there is none. Its hover card says
/// what the server is built with, how lull told, and where the icon came from.
struct ProjectTile: View {
    @Environment(ProjectIcons.self) private var icons: ProjectIcons?
    var cwd: String
    var framework: Framework
    var command: String

    var body: some View {
        let detected = Technology.detect(framework: framework, command: command, mcpEvidence: icons?.mcpEvidence[cwd])
        let icon = icons?.icons[cwd]
        tile(icon, detected?.technology)
            .hoverCard { card(icon, detected) }
            .task(id: cwd) { await icons?.load(cwd) }
    }

    @ViewBuilder private func tile(_ icon: ProjectIcons.Icon?, _ technology: Technology?) -> some View {
        if let icon {
            Group {
                if icon.small {
                    Image(decorative: icon.image, scale: 1).interpolation(.none)
                } else {
                    Image(decorative: icon.image, scale: 2)
                        .resizable()
                        .interpolation(.high)
                        .aspectRatio(contentMode: .fit)
                        .padding(3)
                }
            }
            .iconTile(tint: icon.tint.map { Color(cgColor: $0) })
            .accessibilityLabel(technology.map { TechTile.style($0).name } ?? "project icon")
        } else {
            TechTile(technology: technology)
        }
    }

    /// The icon's path inside the project, else shortened with `~`. The source has its
    /// symlinks resolved (`/private/tmp` reads as `/tmp`), so the cwd is resolved too.
    private func iconPath(_ source: URL) -> String {
        let path = source.path
        for base in [cwd, URL(fileURLWithPath: cwd).resolvingSymlinksInPath().path] where path.hasPrefix(base + "/") {
            return String(path.dropFirst(base.count + 1))
        }
        return Format.tilde(path)
    }

    private func card(_ icon: ProjectIcons.Icon?, _ detected: (technology: Technology, reason: String)?) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            CardTitle(title: detected.map { TechTile.style($0.technology).name } ?? "Unknown technology",
                      subtitle: detected?.reason) {
                TechTile(technology: detected?.technology)
            }
            VStack(alignment: .leading, spacing: 4) {
                if let icon { CardField(label: "Icon", value: iconPath(icon.source)) }
                CardField(label: "Folder", value: Format.tilde(cwd), copy: true)
            }
        }
    }
}
