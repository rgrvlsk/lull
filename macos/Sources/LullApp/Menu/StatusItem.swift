// The menu-bar icon: the cat, optionally followed by the count of running servers and a
// dot while one is stopping soon (both off by default), and a click that toggles the panel
// in a transient popover.
import AppKit
import SwiftUI
import LullKit

@MainActor
final class StatusItem: NSObject {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let popover = NSPopover()
    private let store: Store
    private let icons: ProjectIcons

    init(store: Store, icons: ProjectIcons, openSettings: @escaping () -> Void) {
        self.store = store
        self.icons = icons
        super.init()
        popover.behavior = .transient
        let host = NSHostingController(rootView: MenuView(openSettings: { [popover] in
            popover.performClose(nil)
            openSettings()
        }).environment(store).environment(icons))
        host.sizingOptions = .preferredContentSize
        popover.contentViewController = host
        item.button?.target = self
        item.button?.action = #selector(toggle)
        item.button?.imagePosition = .imageLeading
        item.button?.image = Brand.menuBar
        render()
        // The count and dot preferences live in UserDefaults, which observation does not see.
        NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.draw() }
        }
    }

    @objc private func toggle() {
        guard let button = item.button else { return }
        if popover.isShown {
            popover.performClose(nil)
            return
        }
        Task { await store.refresh() }
        icons.revalidate()
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        if let window = popover.contentViewController?.view.window {
            window.makeKey()
            // Nothing focused or ringed on open; Tab still starts keyboard navigation.
            window.makeFirstResponder(nil)
        }
        paintPopoverFrame()
    }

    /// Paints the popover's frame in the panel colour instead of the translucent material,
    /// and the arrow as the sky continuing above the header. NSPopover has no API for this,
    /// so a view goes under the content.
    private func paintPopoverFrame() {
        guard let content = popover.contentViewController?.view.window?.contentView,
              let frame = content.superview,
              !frame.subviews.contains(where: { $0 is PanelBackground })
        else { return }
        let background = PanelBackground(frame: frame.bounds)
        background.content = content
        background.autoresizingMask = [.width, .height]
        frame.addSubview(background, positioned: .below, relativeTo: content)
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
        guard store.phase == .connected else {
            button.attributedTitle = NSAttributedString()
            button.appearsDisabled = true
            button.toolTip = "lull: daemon not connected"
            return
        }
        button.appearsDisabled = false
        let running = store.servers.values.filter { $0.state != .stopping }.count
        let soon = store.servers.values.contains { $0.state == .stoppingSoon }
        let defaults = UserDefaults.standard
        let showCount = defaults.bool(forKey: MenuBarPrefs.showCount)
        let showDot = defaults.bool(forKey: MenuBarPrefs.showDot)
        let title = NSMutableAttributedString(
            string: Brand.menuBar == nil ? "lull" : showCount && running > 0 ? " \(running)" : "",
            attributes: [.font: NSFont.menuBarFont(ofSize: 0).withMonospacedDigits()])
        if showDot && soon {
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

private final class PanelBackground: NSView {
    weak var content: NSView?

    override init(frame: NSRect) {
        super.init(frame: frame)
        // Everything is placed from the content's top edge, which moves as the panel resizes.
        layerContentsRedrawPolicy = .duringViewResize
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func draw(_ dirtyRect: NSRect) {
        Brand.panel.setFill()
        dirtyRect.fill()
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        guard let content, let band = dark ? Brand.headerDark : Brand.headerLight else { return }
        // The band as `PanelBackdrop` lays it out: scaled to the panel's width, its bottom
        // edge `PanelBackdrop.height` below the content's top. The part its crop hides goes
        // into the arrow, and the band's top row is stretched over whatever arrow is left.
        let box = convert(content.bounds, from: content)
        let height = band.size.height * box.width / band.size.width
        let drawn = NSRect(x: box.minX, y: box.maxY - PanelBackdrop.height, width: box.width, height: height)
        band.draw(in: drawn)
        let topRow = NSRect(x: 0, y: band.size.height - 0.5, width: band.size.width, height: 0.5)
        let above = NSRect(x: drawn.minX, y: drawn.maxY, width: drawn.width, height: bounds.maxY - drawn.maxY)
        if above.height > 0 {
            band.draw(in: above, from: topRow, operation: .sourceOver, fraction: 1)
        }
    }
}

/// App-only display preferences (`@AppStorage` keys), not daemon policy.
enum MenuBarPrefs {
    static let showCount = "menuBar.showRunningCount"
    static let showDot = "menuBar.showStoppingSoonDot"
}

private extension NSFont {
    func withMonospacedDigits() -> NSFont {
        NSFont.monospacedDigitSystemFont(ofSize: pointSize, weight: .regular)
    }
}
