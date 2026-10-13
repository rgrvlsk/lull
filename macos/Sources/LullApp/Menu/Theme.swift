// Panel look: rounded cards with a subtle fill, small uppercase section titles, capsule
// badges. Light and dark get separate values so text stays legible on both.
import SwiftUI
import LullKit

enum Theme {
    static let panelWidth: CGFloat = 380
    static let maxListHeight: CGFloat = 520
    static let corner: CGFloat = 12
}

struct Card<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .background(
                RoundedRectangle(cornerRadius: Theme.corner, style: .continuous)
                    .fill(Color.white.opacity(scheme == .dark ? 0.075 : 0.7))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.corner, style: .continuous)
                    .strokeBorder(Color.primary.opacity(scheme == .dark ? 0.06 : 0.05))
            )
    }
}

struct SectionTitle: View {
    var text: String
    var trailing: String?

    var body: some View {
        HStack {
            Text(text.uppercased())
            Spacer()
            if let trailing { Text(trailing) }
        }
        .font(.system(size: 11, weight: .semibold))
        .tracking(0.6)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 4)
    }
}

struct Badge: View {
    var text: String
    var tint: Color = .secondary

    var body: some View {
        Text(text)
            .font(.system(size: 10, weight: .medium))
            .lineLimit(1)
            .padding(.horizontal, 6)
            .padding(.vertical, 1.5)
            .foregroundStyle(tint)
            .background(Capsule().fill(tint.opacity(0.14)))
    }
}

/// Small rounded tile with the framework's initials, in place of a logo.
struct FrameworkTile: View {
    var framework: Framework

    var body: some View {
        let (label, color) = Self.style(framework)
        Text(label)
            .font(.system(size: 10, weight: .bold, design: .rounded))
            .foregroundStyle(.white)
            .frame(width: 24, height: 24)
            .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(color.gradient))
            .accessibilityLabel(framework.rawValue)
    }

    static func style(_ f: Framework) -> (String, Color) {
        switch f {
        case .vite: ("Vi", .purple)
        case .next: ("Nx", .gray)
        case .astro: ("As", .orange)
        case .nuxt: ("Nu", .green)
        case .remix: ("Rx", .indigo)
        case .wrangler: ("Wr", .orange)
        case .phpBuiltin: ("php", .indigo)
        case .artisan: ("La", .red)
        case .pythonHttp: ("Py", .blue)
        case .rails: ("Rb", .red)
        case .django: ("Dj", .teal)
        case .generic, .unknown: ("</>", .secondary)
        }
    }
}

/// Status colours with separate light and dark values.
enum StatusColor {
    static func running(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(red: 0.40, green: 0.85, blue: 0.50) : Color(red: 0.10, green: 0.55, blue: 0.22)
    }
    static func soon(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(red: 1.00, green: 0.72, blue: 0.30) : Color(red: 0.75, green: 0.42, blue: 0.00)
    }
    static func problem(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(red: 1.00, green: 0.45, blue: 0.42) : Color(red: 0.78, green: 0.15, blue: 0.12)
    }
}
