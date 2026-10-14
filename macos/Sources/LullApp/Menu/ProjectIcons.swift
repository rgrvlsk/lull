// Project icons for the rows, looked up off the main thread and cached by directory.
// Rows show the framework tile until (and unless) their directory has an icon.
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
    @ObservationIgnored private var lookups: [String: ProjectIcon.Lookup] = [:]
    @ObservationIgnored private var pending: Set<String> = []

    /// The rows' @2x pixel size.
    private static let pixels = Int(FrameworkTile.size * 2)

    /// Looks up the icon for `cwd` once; later calls are free.
    func load(_ cwd: String) async {
        guard lookups[cwd] == nil, pending.insert(cwd).inserted else { return }
        let pixels = Self.pixels
        let lookup = await Task.detached(priority: .utility) {
            ProjectIcon.lookup(cwd: cwd, pixels: pixels)
        }.value
        pending.remove(cwd)
        store(lookup, for: cwd)
    }

    // ponytail: re-checked only when the panel opens (stat-only mtimes), so an icon changed
    // while the panel is open shows on the next open; upgrade trigger: users expect icons to
    // update live → watch the search roots with FSEvents.
    /// Looks up again every directory whose files changed since its last lookup.
    func revalidate() {
        let pixels = Self.pixels
        for (cwd, old) in lookups where !pending.contains(cwd) {
            pending.insert(cwd)
            Task {
                let fresh = await Task.detached(priority: .utility) {
                    ProjectIcon.isStale(old, cwd: cwd) ? ProjectIcon.lookup(cwd: cwd, pixels: pixels) : nil
                }.value
                pending.remove(cwd)
                if let fresh { store(fresh, for: cwd) }
            }
        }
    }

    private func store(_ lookup: ProjectIcon.Lookup, for cwd: String) {
        lookups[cwd] = lookup
        guard icons[cwd]?.image !== lookup.image else { return }
        icons[cwd] = lookup.image.map {
            Icon(image: $0, small: lookup.found?.small == true, tint: lookup.tint)
        }
    }
}

/// The project's icon in a rounded square with a light neutral backing, so transparent and
/// dark icons stay visible on both panels. A small icon keeps its own pixel size, crisp, on
/// a tile tinted with its colour. The framework tile when there is none.
struct ProjectTile: View {
    @Environment(ProjectIcons.self) private var icons: ProjectIcons?
    var cwd: String
    var framework: Framework

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
                .frame(width: FrameworkTile.size, height: FrameworkTile.size)
                .background {
                    let tile = RoundedRectangle(cornerRadius: 6, style: .continuous)
                    tile.fill(Color.white.opacity(0.92))
                    if let tint = icon.tint { tile.fill(Color(cgColor: tint).opacity(0.35)) }
                }
                .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Color.black.opacity(0.08)))
                .accessibilityLabel(framework.rawValue)
            } else {
                FrameworkTile(framework: framework)
            }
        }
        .task(id: cwd) { await icons?.load(cwd) }
    }
}
