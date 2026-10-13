// Brand artwork: the vector PDFs in assets/brand, copied into the app bundle by
// scripts/build-app.sh. All of it is single-colour, so it is drawn as a template and tinted.
import AppKit
import SwiftUI

enum Brand {
    static let lavender = Color(red: 0xEE / 255, green: 0xED / 255, blue: 0xF6 / 255)
    static let indigo = Color(red: 0x27 / 255, green: 0x27 / 255, blue: 0x44 / 255)

    /// The cat redrawn for 20 px, as a template image for the menu bar.
    static let menuBar: NSImage? = image("lull-micro-20").map { image in
        image.size = NSSize(width: 20, height: 20)
        image.isTemplate = true
        image.accessibilityDescription = "lull"
        return image
    }

    /// Cat and wordmark side by side.
    static let logo = image("logo-horizontal")

    /// `assets/brand/<name>.pdf` from the app bundle; debug builds outside a bundle
    /// (`swift run`, previews) read it from the checkout.
    private static func image(_ name: String) -> NSImage? {
        if let url = Bundle.main.url(forResource: name, withExtension: "pdf") {
            return NSImage(contentsOf: url)
        }
        #if DEBUG
        let repo = URL(fileURLWithPath: #filePath)  // macos/Sources/LullApp/Brand.swift
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        return NSImage(contentsOf: repo.appendingPathComponent("assets/brand/\(name).pdf"))
        #else
        return nil
        #endif
    }
}
