// Project icons for the rows, looked up off the main thread and cached by directory.
// Rows show the framework tile until (and unless) their directory has an icon.
import AppKit
import SwiftUI
import LullKit

@MainActor
@Observable
final class ProjectIcons {
    /// Decoded icons by server cwd; a cwd without one is absent.
    private(set) var images: [String: CGImage] = [:]
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
        if images[cwd] !== lookup.image { images[cwd] = lookup.image }
    }
}

/// The project's icon in a rounded square with a light neutral backing, so transparent and
/// dark icons stay visible on both panels; the framework tile when there is none.
struct ProjectTile: View {
    @Environment(ProjectIcons.self) private var icons: ProjectIcons?
    var cwd: String
    var framework: Framework

    var body: some View {
        Group {
            if let image = icons?.images[cwd] {
                Image(decorative: image, scale: 2)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
                    .padding(3)
                    .frame(width: FrameworkTile.size, height: FrameworkTile.size)
                    .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color.white.opacity(0.92)))
                    .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Color.black.opacity(0.08)))
                    .accessibilityLabel(framework.rawValue)
            } else {
                FrameworkTile(framework: framework)
            }
        }
        .task(id: cwd) { await icons?.load(cwd) }
    }
}
