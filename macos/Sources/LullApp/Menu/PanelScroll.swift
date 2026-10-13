// The panel's scrolling list. The system indicator would sit over the rows' right column,
// so the list draws its own: a thin capsule centred in the gutter right of the cards,
// shown only when the content overflows, while scrolling or on hover. macOS 14 has no
// `onScrollGeometryChange`, so the offset comes from the content's frame. The top `fade`
// points are an inset at rest, which scrolled content passes through under whatever the
// caller fades over it.
import SwiftUI

private let scrollSpace = "lull.scroll"

struct PanelScroll<Content: View>: View {
    var maxHeight: CGFloat
    /// Width of the gutter between the cards and the panel edge.
    var gutter: CGFloat
    var fade: CGFloat
    @ViewBuilder var content: Content

    @State private var contentHeight: CGFloat = 0
    @State private var offset: CGFloat = 0
    @State private var hovering = false
    @State private var scrolling = false
    @State private var idle: Task<Void, Never>?

    var body: some View {
        let viewport = min(contentHeight, maxHeight)
        ScrollView {
            content.padding(.top, fade).onGeometryChange(for: CGRect.self) { $0.frame(in: .named(scrollSpace)) } action: { frame in
                contentHeight = frame.height
                if offset != -frame.minY {
                    offset = -frame.minY
                    showWhileScrolling()
                }
            }
        }
        .coordinateSpace(.named(scrollSpace))
        .scrollIndicators(.never)
        .frame(height: viewport)
        .overlay(alignment: .topTrailing) { indicator(viewport: viewport) }
        .onHover { hovering = $0 }
    }

    @ViewBuilder private func indicator(viewport: CGFloat) -> some View {
        if contentHeight > viewport + 1 {
            // From the top of the content at rest to just above the bottom edge.
            let track = viewport - fade - 4
            let thumb = max(24, track * viewport / contentHeight)
            let progress = min(max(offset / (contentHeight - viewport), 0), 1)
            Capsule()
                .fill(Color.secondary.opacity(0.4))
                .frame(width: 3, height: thumb)
                .offset(x: -(gutter - 3) / 2, y: fade + (track - thumb) * progress)
                .opacity(hovering || scrolling ? 1 : 0)
                .animation(.easeOut(duration: 0.25), value: hovering || scrolling)
                .allowsHitTesting(false)
        }
    }

    private func showWhileScrolling() {
        scrolling = true
        idle?.cancel()
        idle = Task {
            try? await Task.sleep(for: .seconds(1))
            if !Task.isCancelled { scrolling = false }
        }
    }
}
