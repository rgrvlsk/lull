// Panel look: rounded cards with a subtle fill, small uppercase section titles, capsule
// badges. Light and dark get separate values so text stays legible on both.
import SwiftUI
import LullKit

enum Theme {
    static let panelWidth: CGFloat = 380
    static let maxListHeight: CGFloat = 520
    static let corner: CGFloat = 12
    /// Panel margin, which is also the scroll indicator's gutter.
    static let inset: CGFloat = 14
}

struct Card<Content: View>: View {
    @Environment(\.colorScheme) private var scheme
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .background(
                RoundedRectangle(cornerRadius: Theme.corner, style: .continuous)
                    .fill(Color.white.opacity(scheme == .dark ? 0.075 : 0.6))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Theme.corner, style: .continuous)
                    .strokeBorder(Color.primary.opacity(scheme == .dark ? 0.06 : 0.05))
            )
    }
}

/// Muted text that sits on the panel itself rather than on a card: the system secondary on
/// dark, indigo on the light lavender panel, where neutral grey reads too faint.
enum PanelText {
    static func muted(_ scheme: ColorScheme) -> AnyShapeStyle {
        scheme == .dark ? AnyShapeStyle(.secondary) : AnyShapeStyle(Brand.indigo.opacity(0.75))
    }
}

struct SectionTitle: View {
    @Environment(\.colorScheme) private var scheme
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
        .foregroundStyle(PanelText.muted(scheme))
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

extension View {
    /// The rounded square every row icon sits on: a light neutral backing, so dark and
    /// transparent marks stay visible on both panels, optionally tinted.
    func iconTile(tint: Color? = nil) -> some View {
        frame(width: TechTile.size, height: TechTile.size)
            .background {
                let tile = RoundedRectangle(cornerRadius: 6, style: .continuous)
                tile.fill(Color.white.opacity(0.92))
                if let tint { tile.fill(tint.opacity(0.35)) }
            }
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Color.black.opacity(0.08)))
    }
}

/// The tile of a server whose project has no icon: its technology's mark in the brand's
/// colour, its initials where lull ships no mark (assets/logos/README.md), `</>` when
/// nothing names the technology.
struct TechTile: View {
    static let size: CGFloat = 24
    var technology: Technology?

    var body: some View {
        let style = technology.map(Self.style)
        Group {
            if let style, let mark = style.mark.flatMap({ Brand.mark("tech-\($0)") }) {
                Image(nsImage: mark)
                    .renderingMode(.template)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: 15, height: 15)
                    .foregroundStyle(style.color)
                    .iconTile()
            } else {
                Text(style?.initials ?? "</>")
                    .font(.system(size: 10, weight: .bold, design: .rounded))
                    .foregroundStyle(.white)
                    .frame(width: Self.size, height: Self.size)
                    .background(RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .fill((style?.color ?? Color.secondary).gradient))
            }
        }
        .accessibilityLabel(technology?.rawValue ?? "server")
    }

    struct Style {
        var mark: String?
        var color: Color
        var initials: String
    }

    static func style(_ technology: Technology) -> Style {
        func style(_ mark: String?, _ hex: UInt32, _ initials: String) -> Style {
            Style(mark: mark, color: Color(red: Double(hex >> 16 & 0xFF) / 255, green: Double(hex >> 8 & 0xFF) / 255,
                                           blue: Double(hex & 0xFF) / 255), initials: initials)
        }
        return switch technology {
        case .vite: style("vite", 0x9135FF, "Vi")
        case .next: style("next", 0x000000, "Nx")
        case .astro: style("astro", 0xBC52EE, "As")
        case .nuxt: style("nuxt", 0x00DC82, "Nu")
        case .remix: style("remix", 0x000000, "Rx")
        case .wrangler: style("wrangler", 0xF38020, "Wr")
        case .laravel: style("laravel", 0xFF2D20, "La")
        case .rails: style("rails", 0xD30001, "Rl")
        case .django: style("django", 0x092E20, "Dj")
        case .mcp: style("mcp", 0x000000, "MCP")
        case .node: style("node", 0x5FA04E, "JS")
        case .bun: style("bun", 0x000000, "Bun")
        case .deno: style("deno", 0x000000, "De")
        case .python: style("python", 0x3776AB, "Py")
        case .go: style("go", 0x00ADD8, "Go")
        case .elixir: style("elixir", 0x4B275F, "Ex")
        case .php: style(nil, 0x777BB4, "php")
        case .ruby: style(nil, 0xCC342D, "Rb")
        case .rust: style(nil, 0xB7410E, "Rs")
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
