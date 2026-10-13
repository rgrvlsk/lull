// The panel's header: the logo, with the status pill beside it. Kept apart from the list
// so its design can change on its own.
import SwiftUI

struct PanelHeader<Status: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var status: Status

    var body: some View {
        HStack(spacing: 14) {
            if let logo = Brand.logo {
                Image(nsImage: logo)
                    .renderingMode(.template)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(height: 30)
                    .foregroundStyle(scheme == .dark ? Brand.lavender : Brand.indigo)
                    .accessibilityLabel("lull")
            } else {
                Text("lull").font(.system(size: 17, weight: .bold))
            }
            status
            Spacer()
        }
    }
}
