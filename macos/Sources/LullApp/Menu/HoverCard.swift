// Hover cards: a small floating panel with an element's details, shown once the pointer
// rests on it and kept open while the pointer moves into the card, so its buttons can be
// clicked. lull manages the window itself: `.help` takes only plain text, and a popover
// inside the panel's popover would take its focus. One card at a time, each after the
// pointer has rested on its element for a second.
import AppKit
import SwiftUI

extension View {
    func hoverCard<Card: View>(@ViewBuilder _ card: @escaping () -> Card) -> some View {
        modifier(HoverCardTrigger(card: { AnyView(card()) }))
    }
}

/// The view a card is shown for.
@MainActor final class CardAnchor {
    weak var view: NSView?
}

private struct HoverCardTrigger: ViewModifier {
    var card: () -> AnyView
    @State private var anchor = CardAnchor()

    func body(content: Content) -> some View {
        content
            .background(AnchorView(anchor: anchor))
            .onHover { inside in
                if inside {
                    HoverCards.shared.enter(anchor, card: card)
                } else {
                    HoverCards.shared.exit(anchor)
                }
            }
            .onDisappear { HoverCards.shared.exit(anchor) }
    }
}

/// Registers itself as its anchor's view while it is in a window. SwiftUI can briefly keep
/// two of these for one anchor (when the tile swaps the framework mark for the project's
/// icon), so the one that leaves the window last must not win.
private final class AnchorNSView: NSView {
    weak var anchor: CardAnchor?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            anchor?.view = self
        } else if anchor?.view === self {
            anchor?.view = nil
        }
    }
}

private struct AnchorView: NSViewRepresentable {
    let anchor: CardAnchor
    func makeNSView(context: Context) -> AnchorNSView {
        let view = AnchorNSView()
        view.anchor = anchor
        return view
    }
    func updateNSView(_ view: AnchorNSView, context: Context) { view.anchor = anchor }
}

@MainActor final class HoverCards {
    static let shared = HoverCards()

    private let panel: NSPanel = {
        let panel = NSPanel(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                            backing: .buffered, defer: true)
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.popUpMenu.rawValue + 1)
        panel.hidesOnDeactivate = false
        panel.alphaValue = 0
        panel.isReleasedWhenClosed = false
        return panel
    }()

    private var shown: CardAnchor?
    private var hovered: CardAnchor?
    private var overCard = false
    private var opening: Task<Void, Never>?
    private var closing: Task<Void, Never>?
    /// Bumped by every show and close, so a fade-out that finishes after the card came
    /// back does not take it away.
    private var generation = 0

    private init() {
        // The panel's popover closing (click outside, Escape, an action) takes its card along.
        NotificationCenter.default.addObserver(forName: NSPopover.willCloseNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { HoverCards.shared.dismiss(animated: false) }
        }
        // While a card is attached the popover misses Escape; removing the card first lets the
        // key go on to close the panel as usual.
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated {
                if event.keyCode == 53, HoverCards.shared.panel.isVisible { HoverCards.shared.dismiss(animated: false) }
            }
            return event
        }
    }

    /// Every card waits this long, also when another is open: passing over a mark on the
    /// way elsewhere opens nothing.
    private static let openDelay = Duration.seconds(1)
    private static let closeDelay = Duration.milliseconds(200)

    func enter(_ anchor: CardAnchor, card: @escaping () -> AnyView) {
        hovered = anchor
        opening?.cancel()
        if shown === anchor, panel.isVisible {
            closing?.cancel()
            return
        }
        // Another element's card closes as if the pointer had left it; this one waits its turn.
        if panel.isVisible { scheduleClose() }
        opening = Task {
            try? await Task.sleep(for: Self.openDelay)
            guard !Task.isCancelled, hovered === anchor else { return }
            show(card(), at: anchor)
        }
    }

    func exit(_ anchor: CardAnchor) {
        guard hovered === anchor else { return }
        hovered = nil
        opening?.cancel()
        scheduleClose()
    }

    /// Closes the card now: the list scrolled under it, or (without the fade) the panel is
    /// closing.
    func dismiss(animated: Bool = true) {
        opening?.cancel()
        closing?.cancel()
        hovered = nil
        overCard = false
        close(animated: animated)
    }

    /// Closes the card shortly unless the pointer is back on its element or inside it.
    private func scheduleClose() {
        closing?.cancel()
        closing = Task {
            try? await Task.sleep(for: Self.closeDelay)
            guard !Task.isCancelled, shown == nil || hovered !== shown, !overCard else { return }
            close()
        }
    }

    private func cardHover(_ inside: Bool) {
        overCard = inside
        if inside {
            closing?.cancel()
        } else {
            scheduleClose()
        }
    }

    private func show(_ card: AnyView, at anchor: CardAnchor) {
        guard let view = anchor.view, let window = view.window, window.isVisible else { return }
        let host = CardHostingView(rootView: AnyView(
            CardChrome(content: card).onHover { [weak self] in self?.cardHover($0) }))
        panel.appearance = window.effectiveAppearance
        panel.contentView = host
        // Sized for the widest card, as tall as its content needs.
        let fitting = NSHostingController(rootView: host.rootView)
            .sizeThatFits(in: NSSize(width: CardChrome.width, height: 10_000))
        let size = NSSize(width: ceil(fitting.width), height: ceil(fitting.height))
        let rect = window.convertToScreen(view.convert(view.bounds, to: nil))
        // Below the element, aligned with its left edge; above it when there is no room.
        var origin = NSPoint(x: rect.minX - 6, y: rect.minY - 6 - size.height)
        if let screen = window.screen?.visibleFrame {
            if origin.y < screen.minY { origin.y = rect.maxY + 6 }
            origin.x = min(max(origin.x, screen.minX + 4), screen.maxX - size.width - 4)
        }
        let frame = NSRect(origin: origin, size: size)
        if panel.parent !== window {
            panel.parent?.removeChildWindow(panel)
            window.addChildWindow(panel, ordered: .above)
        }
        generation += 1
        if panel.isVisible && panel.alphaValue == 1 {
            panel.setFrame(frame, display: true)
        } else {
            // Fades in while sliding the last few points away from its element; with Reduce
            // Motion it only fades.
            let slide: CGFloat = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? 0 : 4
            panel.setFrame(frame.offsetBy(dx: 0, dy: origin.y < rect.minY ? slide : -slide), display: true)
            panel.orderFront(nil)
            NSAnimationContext.runAnimationGroup { context in
                context.duration = Self.fadeIn
                context.timingFunction = CAMediaTimingFunction(name: .easeOut)
                panel.animator().alphaValue = 1
                panel.animator().setFrame(frame, display: true)
            }
        }
        shown = anchor
    }

    private static let fadeIn: TimeInterval = 0.16
    private static let fadeOut: TimeInterval = 0.12

    private func close(animated: Bool = true) {
        shown = nil
        generation += 1
        let closing = generation
        guard animated, panel.isVisible else { return removePanel() }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = Self.fadeOut
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().alphaValue = 0
        } completionHandler: {
            MainActor.assumeIsolated {
                if HoverCards.shared.generation == closing { HoverCards.shared.removePanel() }
            }
        }
    }

    private func removePanel() {
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        panel.alphaValue = 0
        panel.contentView = nil
    }
}

/// Clicks land on the first try although the card's window is never key.
private final class CardHostingView: NSHostingView<AnyView> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// The card's frame: the panel colour, rounded, with a hairline edge.
private struct CardChrome: View {
    static let width: CGFloat = 280
    var content: AnyView

    var body: some View {
        content
            .font(.system(size: 11))
            .padding(10)
            .frame(width: Self.width, alignment: .leading)
            .background(Color(nsColor: Brand.panel), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color.primary.opacity(0.12)))
    }
}

/// A card's title row: a mark and a name, with a muted subtitle.
struct CardTitle<Mark: View>: View {
    var title: String
    var subtitle: String?
    @ViewBuilder var mark: Mark

    var body: some View {
        HStack(spacing: 8) {
            mark
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.system(size: 12, weight: .semibold))
                if let subtitle { Text(subtitle).foregroundStyle(.secondary) }
            }
        }
    }
}

/// A labelled value in a card, monospaced, with a copy button when `copy` is set.
struct CardField: View {
    var label: String
    var value: String
    var copy = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(label).foregroundStyle(.secondary)
            Text(value)
                .font(.system(size: 11, design: .monospaced))
                .lineLimit(2)
                .truncationMode(.middle)
                .textSelection(.enabled)
            if copy {
                Spacer(minLength: 0)
                IconButton(symbol: "doc.on.doc", help: "Copy \(label.lowercased())") { System.copy(value) }
                    .scaleEffect(0.85)
            }
        }
    }
}
