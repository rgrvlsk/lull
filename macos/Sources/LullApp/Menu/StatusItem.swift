// The menu-bar icon: the count of running servers, a dot while one is stopping soon, and
// a click that toggles the panel in a transient popover.
import AppKit
import SwiftUI
import LullKit

@MainActor
final class StatusItem: NSObject {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private let store: Store

    init(store: Store, openSettings: @escaping () -> Void) {
        self.store = store
        super.init()
        popover.behavior = .transient
        let host = NSHostingController(rootView: MenuView(openSettings: { [popover] in
            popover.performClose(nil)
            openSettings()
        }).environment(store))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        item.button?.target = self
        item.button?.action = #selector(toggle)
        item.button?.imagePosition = .imageLeading
        render()
    }

    @objc private func toggle() {
        guard let button = item.button else { return }
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        Task { await store.refresh() }
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        popover.contentViewController?.view.window?.makeKey()
    }

    /// Redraws the button whenever what it shows changes.
    private func render() {
        withObservationTracking {
            draw()
        } onChange: { [weak self] in
            Task { @MainActor in self?.render() }
        }
    }

    private func draw() {
        guard let button = item.button else { return }
        let image = NSImage(systemSymbolName: "server.rack", accessibilityDescription: "lull")
        image?.isTemplate = true
        button.image = image
        guard store.phase == .connected else {
            button.attributedTitle = NSAttributedString()
            button.appearsDisabled = true
            button.toolTip = "lull: daemon not connected"
            return
        }
        button.appearsDisabled = false
        let running = store.servers.values.filter { $0.state != .stopping }.count
        let soon = store.servers.values.contains { $0.state == .stoppingSoon }
        let title = NSMutableAttributedString(
            string: running > 0 ? " \(running)" : "",
            attributes: [.font: NSFont.menuBarFont(ofSize: 0).withMonospacedDigits()])
        if soon {
            title.append(NSAttributedString(string: " ●", attributes: [
                .foregroundColor: NSColor.systemOrange,
                .font: NSFont.systemFont(ofSize: 8),
                .baselineOffset: 2,
            ]))
        }
        button.attributedTitle = title
        button.toolTip = "lull: \(running) running" + (soon ? ", stopping soon" : "")
    }
}

private extension NSFont {
    func withMonospacedDigits() -> NSFont {
        NSFont.monospacedDigitSystemFont(ofSize: pointSize, weight: .regular)
    }
}
