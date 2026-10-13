// The panel's header: the skyline band across the top, the logo floating on it and the
// status pill on the right. Kept apart from the list so its design can change on its own.
import SwiftUI

struct PanelHeader<Status: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var status: Status

    var body: some View {
        let dark = scheme == .dark
        ZStack(alignment: .topLeading) {
            if let band = dark ? Brand.headerDark : Brand.headerLight {
                // Scaled to the panel's width; the top is cropped so the faded bottom edge,
                // which is exactly the panel colour, stays whole.
                Image(nsImage: band)
                    .resizable()
                    .scaledToFill()
                    .frame(width: Theme.panelWidth, height: 100, alignment: .bottom)
                    .clipped()
                    .accessibilityHidden(true)
            }
            HStack(alignment: .center) {
                if let logo = Brand.logo {
                    Image(nsImage: logo)
                        .renderingMode(.template)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(height: 28)
                        .foregroundStyle(dark ? Brand.lavender : Brand.indigo)
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
        .frame(width: Theme.panelWidth, height: 100, alignment: .topLeading)
    }
}
