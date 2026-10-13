// The panel's header: the skyline band across the top (`PanelBackdrop`), behind the list,
// and the logo with the status pill floating above everything (`PanelHeader`). Kept apart
// from the list so its design can change on its own.
import SwiftUI

struct PanelBackdrop: View {
    static let height: CGFloat = 100
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if let band = scheme == .dark ? Brand.headerDark : Brand.headerLight {
            // Scaled to the panel's width; the top is cropped so the faded bottom edge,
            // which is exactly the panel colour, stays whole.
            Image(nsImage: band)
                .resizable()
                .scaledToFill()
                .frame(width: Theme.panelWidth, height: Self.height, alignment: .bottom)
                .clipped()
                .accessibilityHidden(true)
        }
    }
}

struct PanelHeader<Status: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var status: Status

    var body: some View {
        HStack(alignment: .center) {
            if let logo = Brand.logo {
                Image(nsImage: logo)
                    .renderingMode(.template)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(height: 28)
                    .foregroundStyle(scheme == .dark ? Brand.lavender : Brand.indigo)
                    .accessibilityLabel("lull")
            } else {
                Text("lull").font(.system(size: 17, weight: .bold))
            }
            Spacer()
            status
        }
        .padding(.top, 14)
        .padding(.horizontal, 16)
    }
}
