// Hover cards: a small floating panel with an element's details, shown once the pointer
// rests on it and kept open while the pointer moves into the card, so its buttons can be
// clicked. lull manages the window itself: `.help` takes only plain text, and a popover
// inside the panel's popover would take its focus. One card at a time; moving to another
// element, or onto one shortly after a card closed, swaps it at once.
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
        panel.isReleasedWhenClosed = false
        return panel
    }()

    private var shown: CardAnchor?
    private var hovered: CardAnchor?
    private var overCard = false
    private var pending: Task<Void, Never>?
    private var closedAt: ContinuousClock.Instant?

    private init() {
        // The panel's popover closing (click outside, Escape, an action) takes its card along.
        NotificationCenter.default.addObserver(forName: NSPopover.willCloseNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { HoverCards.shared.dismiss() }
        }
        // While a card is attached the popover misses Escape; closing the card first lets the
        // key go on to close the panel as usual.
        NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            MainActor.assumeIsolated {
                if event.keyCode == 53, HoverCards.shared.panel.isVisible { HoverCards.shared.dismiss() }
            }
            return event
        }
    }

    /// Long enough that passing over a mark on the way elsewhere opens nothing.
    private static let openDelay = Duration.seconds(1)
    private static let closeDelay = Duration.milliseconds(200)
    /// After a card closes, the next one opens at once for this long, like system tooltips.
    private static let grace = Duration.milliseconds(600)

    func enter(_ anchor: CardAnchor, card: @escaping () -> AnyView) {
        hovered = anchor
        pending?.cancel()
        guard shown !== anchor || !panel.isVisible else { return }
        let recent = closedAt.map { ContinuousClock.now - $0 < Self.grace } ?? false
        let delay = panel.isVisible || recent ? Duration.zero : Self.openDelay
        pending = Task {
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled, hovered === anchor else { return }
            show(card(), at: anchor)
        }
    }

    func exit(_ anchor: CardAnchor) {
        guard hovered === anchor else { return }
        hovered = nil
        scheduleClose()
    }

    /// Closes the card now: the panel closed, or the list scrolled under it.
    func dismiss() {
        pending?.cancel()
        hovered = nil
        overCard = false
        close()
    }

    private func scheduleClose() {
        pending?.cancel()
        pending = Task {
            try? await Task.sleep(for: Self.closeDelay)
            guard !Task.isCancelled, hovered == nil, !overCard else { return }
            close()
        }
    }

    private func cardHover(_ inside: Bool) {
        overCard = inside
        if !inside && hovered == nil { scheduleClose() }
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
        panel.setFrame(NSRect(origin: origin, size: size), display: true)
        if panel.parent !== window {
            panel.parent?.removeChildWindow(panel)
            window.addChildWindow(panel, ordered: .above)
        }
        panel.orderFront(nil)
        shown = anchor
    }

    private func close() {
        if panel.isVisible { closedAt = .now }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        panel.contentView = nil
        shown = nil
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
