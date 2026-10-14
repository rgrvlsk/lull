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
    }

    /// Decoded icons by server cwd; a cwd without one is absent.
    private(set) var icons: [String: Icon] = [:]
    /// Server cwds whose project is an MCP server (`Technology.isMCPProject`).
    private(set) var mcpProjects: Set<String> = []
    @ObservationIgnored private var lookups: [String: ProjectIcon.Lookup] = [:]
    @ObservationIgnored private var pending: Set<String> = []

    /// The rows' @2x pixel size.
    private static let pixels = Int(TechTile.size * 2)

    /// Looks up the icon for `cwd` once; later calls are free.
    func load(_ cwd: String) async {
        guard lookups[cwd] == nil, pending.insert(cwd).inserted else { return }
        let pixels = Self.pixels
        let (lookup, mcp) = await Task.detached(priority: .utility) {
            (ProjectIcon.lookup(cwd: cwd, pixels: pixels), Technology.isMCPProject(cwd: cwd))
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
                     Technology.isMCPProject(cwd: cwd))
                }.value
                pending.remove(cwd)
                if let fresh { store(fresh, for: cwd) }
                store(mcp: mcp, for: cwd)
            }
        }
    }

    private func store(mcp: Bool, for cwd: String) {
        if mcp != mcpProjects.contains(cwd) { mcpProjects.formSymmetricDifference([cwd]) }
    }

    private func store(_ lookup: ProjectIcon.Lookup, for cwd: String) {
        lookups[cwd] = lookup
        guard icons[cwd]?.image !== lookup.image else { return }
        icons[cwd] = lookup.image.map {
            Icon(image: $0, small: lookup.found?.small == true, tint: lookup.tint)
        }
    }
}

/// The project's icon on the row tile. A small icon keeps its own pixel size, crisp, on a
/// tile tinted with its colour. The technology tile when there is none.
struct ProjectTile: View {
    @Environment(ProjectIcons.self) private var icons: ProjectIcons?
    var cwd: String
    var framework: Framework
    var command: String

    var body: some View {
        Group {
            if let icon = icons?.icons[cwd] {
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
                .accessibilityLabel(framework.rawValue)
            } else {
                TechTile(technology: Technology.of(
                    framework: framework, command: command, mcpProject: icons?.mcpProjects.contains(cwd) == true))
            }
        }
        .task(id: cwd) { await icons?.load(cwd) }
    }
}
