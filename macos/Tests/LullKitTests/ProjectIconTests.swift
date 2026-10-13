// The project icon resolver against made-up projects in a temporary directory.
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import LullKit

@Suite final class ProjectIconTests {
    let root: URL

    init() throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("lull-icon-\(UUID().uuidString)")
            .resolvingSymlinksInPath()
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    deinit { try? FileManager.default.removeItem(at: root) }

    private func write(_ path: String, _ text: String) throws {
        let url = root.appendingPathComponent(path)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: url)
    }

    private func png(_ path: String, _ width: Int, _ height: Int) throws {
        let url = root.appendingPathComponent(path)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let context = try #require(CGContext(
            data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(red: 0.2, green: 0.4, blue: 0.8, alpha: 1)
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        let image = try #require(context.makeImage())
        let destination = try #require(
            CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
        CGImageDestinationAddImage(destination, image, nil)
        #expect(CGImageDestinationFinalize(destination))
    }

    private let svg = #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><circle cx="32" cy="32" r="30"/></svg>"#

    private func found() -> String? {
        ProjectIcon.find(in: root).map { String($0.url.path.dropFirst(root.path.count + 1)) }
    }

    @Test func htmlLinkPicksTheBestMatchingSizes() throws {
        try png("public/icon-16.png", 16, 16)
        try png("public/icon-64.png", 64, 64)
        try png("public/icon-512.png", 512, 512)
        try write("index.html", """
            <html><head>
            <link rel="icon" sizes="16x16" href="/icon-16.png">
            <LINK href='/icon-512.png' rel='icon' sizes='512x512'>
            <link rel="shortcut icon" sizes="64x64" href="/icon-64.png?v=3">
            </head></html>
            """)
        #expect(found() == "public/icon-64.png")
    }

    @Test func manifestPicksTheRightSize() throws {
        try png("public/android-192.png", 192, 192)
        try png("public/android-48.png", 48, 48)
        try write("public/site.webmanifest", #"""
            {"name":"example-app","icons":[
              {"src":"android-192.png","sizes":"192x192","type":"image/png"},
              {"src":"/android-48.png","sizes":"48x48","type":"image/png"}]}
            """#)
        #expect(found() == "public/android-48.png")
    }

    @Test func nextAppIconSvg() throws {
        try write("app/icon.svg", svg)
        try png("public/favicon-32x32.png", 32, 32)
        #expect(found() == "app/icon.svg")
    }

    @Test func aSixteenPixelFaviconIsRejected() throws {
        try png("public/favicon.png", 16, 16)
        #expect(found() == nil)
    }

    @Test func aWideLogoIsRejected() throws {
        try png("public/icon.png", 200, 50)
        try png("public/favicon.png", 64, 64)
        #expect(found() == "public/favicon.png")
    }

    @Test func anHrefLeavingTheRootIsRefused() throws {
        let outside = root.deletingLastPathComponent().appendingPathComponent("lull-icon-outside-\(UUID().uuidString).svg")
        try Data(svg.utf8).write(to: outside)
        defer { try? FileManager.default.removeItem(at: outside) }
        try write("index.html", #"<link rel="icon" href="../\#(outside.lastPathComponent)">"#)
        #expect(found() == nil)
    }

    @Test func remoteAndDataHrefsAreSkipped() throws {
        try write("index.html", """
            <link rel="icon" href="https://example.com/favicon.svg">
            <link rel="icon" href="//example.com/favicon.svg">
            <link rel="icon" href="data:image/svg+xml,abc">
            <link rel="mask-icon" href="/pin.svg">
            """)
        try write("public/pin.svg", svg)
        #expect(found() == nil)
    }

    @Test func lookupDecodesAThumbnailAndNoticesChanges() throws {
        try png("public/icon.png", 1024, 1024)
        let lookup = ProjectIcon.lookup(cwd: root.path, pixels: 48)
        #expect(lookup.image?.width == 48)
        #expect(!ProjectIcon.isStale(lookup, cwd: root.path))
        try png("public/favicon.svg.tmp", 64, 64)
        #expect(ProjectIcon.isStale(lookup, cwd: root.path))
    }
}
