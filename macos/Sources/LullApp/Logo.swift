// The lull logo, sleeping cat and wordmark, drawn from the same half-block pixel grid as
// `LOGO` in crates/cli/src/ui/mod.rs (copied here; change both together). Each character
// is one pixel column and two pixel rows: `▀` top, `▄` bottom, `█` both.
import AppKit

enum Logo {
    static let rows = [
        "          ▄█▄     ▄",
        "    ▄▄███████▄▄ ▄██     ██         ██ ██",
        "  ▄████████████████     ██ ▄▄   ▄▄ ██ ██",
        " ███▄▄▄▀███▀▀██████     ██ ██   ██ ██ ██",
        " ███████▄▀█████▀▀██     ██ ██   ██ ██ ██",
        " ▀███████▄▀▀▀▀▀██▀      ██ ▀█████▀ ██ ██",
        "   ▀▀█████████▀",
    ]
    /// Pixel columns of the cat alone (column 0 is empty), and of the cat with the wordmark.
    static let cat = 1..<19
    static let all = 1..<39

    /// Brand colours: lavender on an indigo plate.
    static let lavender = NSColor(srgbRed: 0xEE / 255, green: 0xED / 255, blue: 0xF6 / 255, alpha: 1)
    static let indigo = NSColor(srgbRed: 0x27 / 255, green: 0x27 / 255, blue: 0x44 / 255, alpha: 1)

    /// `pixels[y][x]`, 14 rows by 40 columns, top row first.
    static let pixels: [[Bool]] = rows.flatMap { row -> [[Bool]] in
        let chars = Array(row.padding(toLength: 40, withPad: " ", startingAt: 0))
        return [chars.map { $0 == "▀" || $0 == "█" }, chars.map { $0 == "▄" || $0 == "█" }]
    }

    /// The pixels of `columns`, `pixel` points each, on a transparent background, in
    /// `color`. Edges are snapped to device pixels so the art stays crisp.
    static func draw(columns: Range<Int>, pixel: CGFloat, color: NSColor, origin: CGPoint) {
        let scale = NSGraphicsContext.current?.cgContext.userSpaceToDeviceSpaceTransform.a ?? 2
        func snap(_ v: CGFloat) -> CGFloat { (v * scale).rounded() / scale }
        color.setFill()
        for (y, row) in pixels.enumerated() {
            for x in columns where row[x] {
                let x0 = snap(origin.x + CGFloat(x - columns.lowerBound) * pixel)
                let y0 = snap(origin.y + CGFloat(y) * pixel)
                let x1 = snap(origin.x + CGFloat(x - columns.lowerBound + 1) * pixel)
                let y1 = snap(origin.y + CGFloat(y + 1) * pixel)
                NSRect(x: x0, y: y0, width: x1 - x0, height: y1 - y0).fill()
            }
        }
    }

    /// The cat alone as a template image, `height` points tall, for the menu bar: macOS
    /// tints it for light and dark menu bars.
    static func menuBarImage(height: CGFloat = 17.5) -> NSImage {
        let pixel = height / CGFloat(pixels.count)
        let size = NSSize(width: (CGFloat(cat.count) * pixel).rounded(.up), height: height)
        let image = NSImage(size: size, flipped: true) { _ in
            draw(columns: cat, pixel: pixel, color: .black, origin: .zero)
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "lull"
        return image
    }

    /// `columns` in lavender on a rounded indigo plate, with `inset` pixels of margin.
    static func plate(columns: Range<Int>, pixel: CGFloat, inset: CGFloat = 3) -> NSImage {
        let margin = inset * pixel
        let size = NSSize(width: CGFloat(columns.count) * pixel + 2 * margin,
                          height: CGFloat(pixels.count) * pixel + 2 * margin)
        return NSImage(size: size, flipped: true) { rect in
            indigo.setFill()
            NSBezierPath(roundedRect: rect, xRadius: margin * 1.2, yRadius: margin * 1.2).fill()
            draw(columns: columns, pixel: pixel, color: lavender, origin: CGPoint(x: margin, y: margin))
            return true
        }
    }

    /// The app icon at `side` pixels: the cat on the indigo plate, inside the standard
    /// macOS icon margin.
    static func appIcon(side: CGFloat) -> NSImage {
        NSImage(size: NSSize(width: side, height: side), flipped: true) { _ in
            let plate = side * 0.805
            let inset = (side - plate) / 2
            indigo.setFill()
            NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: plate, height: plate),
                         xRadius: plate * 0.225, yRadius: plate * 0.225).fill()
            // Whole device pixels per logo pixel when the icon is large enough.
            let raw = plate * 0.62 / CGFloat(cat.count)
            let pixel = raw >= 1 ? raw.rounded(.down) : raw
            let w = CGFloat(cat.count) * pixel, h = CGFloat(pixels.count) * pixel
            draw(columns: cat, pixel: pixel, color: lavender,
                 origin: CGPoint(x: ((side - w) / 2).rounded(), y: ((side - h) / 2).rounded()))
            return true
        }
    }

    /// Writes an `.iconset` directory for `iconutil -c icns` (used by build-app.sh), so the
    /// bundle icon comes from the same grid and no image is committed.
    static func writeIconset(to dir: URL) throws {
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        for points in [16, 32, 128, 256, 512] {
            for scale in [1, 2] {
                let side = points * scale
                let rep = NSBitmapImageRep(
                    bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8,
                    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                    bytesPerRow: 0, bitsPerPixel: 0)!
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
                NSGraphicsContext.current?.imageInterpolation = .none
                appIcon(side: CGFloat(side)).draw(in: NSRect(x: 0, y: 0, width: side, height: side))
                NSGraphicsContext.restoreGraphicsState()
                let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
                try rep.representation(using: .png, properties: [:])!.write(to: dir.appendingPathComponent(name))
            }
        }
    }
}
