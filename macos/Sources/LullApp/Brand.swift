// Brand artwork from assets/brand, copied into the app bundle by scripts/build-app.sh: the
// single-colour vector PDFs, drawn as templates and tinted, and the panel header's skyline.
import AppKit
import SwiftUI

enum Brand {
    static let lavender = Color(red: 0xEE / 255, green: 0xED / 255, blue: 0xF6 / 255)
    static let indigo = Color(red: 0x27 / 255, green: 0x27 / 255, blue: 0x44 / 255)

    /// The panel's solid background: the darkest buildings of each skyline, `#20203A` dark
    /// and `#BDBAD2` light, so the header melts into the panel.
    static let panel = NSColor(name: "lull.panel") { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(srgbRed: 0x20 / 255, green: 0x20 / 255, blue: 0x3A / 255, alpha: 1)
            : NSColor(srgbRed: 0xBD / 255, green: 0xBA / 255, blue: 0xD2 / 255, alpha: 1)
    }

    /// The cat redrawn for 20 px, as a template image for the menu bar.
    static let menuBar: NSImage? = image("lull-micro-20").map { image in
        image.size = NSSize(width: 20, height: 20)
        image.isTemplate = true
        image.accessibilityDescription = "lull"
        return image
    }

    /// Cat and wordmark side by side.
    static let logo = image("logo-horizontal")

    /// The night-sky skyline behind the panel header, a 360 × 100 pt band without the logo,
    /// opaque over `panel`.
    static let headerDark = header("header-dark")
    static let headerLight = header("header-light")

    private static func header(_ name: String) -> NSImage? {
        let image = NSImage(size: NSSize(width: 360, height: 100))
        for scale in ["@2x", "@4x"] {
            if let url = url("header/\(name)\(scale).png"), let rep = NSImageRep(contentsOf: url) {
                rep.size = image.size
                image.addRepresentation(rep)
            }
        }
        return image.representations.isEmpty ? nil : image
    }

    private static func image(_ name: String) -> NSImage? {
        url("\(name).pdf").flatMap(NSImage.init(contentsOf:))
    }

    /// A third-party mark from `assets/logos` (see its README), cached.
    @MainActor static func mark(_ name: String) -> NSImage? {
        if let image = marks[name] { return image }
        let image = url("\(name).svg", in: "logos").flatMap(NSImage.init(contentsOf:))
        marks[name] = image
        return image
    }

    @MainActor private static var marks: [String: NSImage?] = [:]

    /// `assets/<dir>/<path>` from the app bundle's Resources (by file name); debug builds
    /// outside a bundle (`swift run`, previews) read it from the checkout.
    private static func url(_ path: String, in dir: String = "brand") -> URL? {
        let name = (path as NSString).lastPathComponent
        if let url = Bundle.main.resourceURL?.appendingPathComponent(name),
           FileManager.default.fileExists(atPath: url.path) {
            return url
        }
        #if DEBUG
        let repo = URL(fileURLWithPath: #filePath)  // macos/Sources/LullApp/Brand.swift
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        return repo.appendingPathComponent("assets/\(dir)/\(path)")
        #else
        return nil
        #endif
    }
}
