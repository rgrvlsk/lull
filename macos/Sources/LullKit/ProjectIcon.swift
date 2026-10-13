// A server's project icon, read from the project's files (contract §10). Never fetched from
// the server: lull makes no network requests (§14), and a request would count as traffic.
//
// Search roots are the server's cwd, then its git root. In each, the first candidate that
// passes the size check wins: icons the project declares (HTML `<link rel=icon>`, web
// manifests, Next.js `app/` files), then conventional file names in conventional
// directories. File contents are untrusted: only ImageIO and a minimal attribute scan read
// them, and no path may leave its search root.
import AppKit
import Foundation
import ImageIO

public enum ProjectIcon {
    /// An image file that passed the size check, and which of its images to draw (ICO files
    /// hold several).
    public struct Found: Equatable, Sendable {
        public var url: URL
        public var index: Int
        public var vector: Bool
    }

    /// What `lookup` returns: the decoded icon, if any, and the modification times that
    /// decide whether it needs looking up again.
    public struct Lookup: @unchecked Sendable {
        public var image: CGImage?
        public var found: Found?
        public var stamp: [Double]
    }

    static let maxBytes = 1 << 20
    static let minPixels = 32
    static let maxAspect = 1.5

    // MARK: Lookup

    /// Finds and decodes the icon for a server in `cwd`, at most `pixels` on its long side.
    public static func lookup(cwd: String, pixels: Int) -> Lookup {
        let roots = searchRoots(cwd: cwd)
        let found = roots.lazy.compactMap(find(in:)).first
        return Lookup(image: found.flatMap { image($0, pixels: pixels) }, found: found,
                      stamp: stamp(roots: roots, found: found))
    }

    /// Whether any file `lookup` looked at has changed since it made `old`. Only stats files.
    public static func isStale(_ old: Lookup, cwd: String) -> Bool {
        stamp(roots: searchRoots(cwd: cwd), found: old.found) != old.stamp
    }

    /// The cwd, then the git root above it if that differs, with symlinks resolved.
    public static func searchRoots(cwd: String) -> [URL] {
        let start = URL(fileURLWithPath: cwd).standardizedFileURL.resolvingSymlinksInPath()
        var dir = start
        while dir.path != "/" {
            if FileManager.default.fileExists(atPath: dir.appendingPathComponent(".git").path) {
                return dir == start ? [start] : [start, dir]
            }
            dir.deleteLastPathComponent()
        }
        return [start]
    }

    /// The first candidate under `root` that passes the size check.
    public static func find(in root: URL) -> Found? {
        for candidate in declared(in: root) + conventional(in: root) {
            if let found = check(candidate, root: root) { return found }
        }
        return nil
    }

    // MARK: Candidates

    static let htmlFiles = ["index.html", "public/index.html", "src/index.html"]
    static let manifestFiles = ["manifest.json", "site.webmanifest", "manifest.webmanifest"]
    static let manifestDirs = ["", "public", "static", "app"]
    static let nextDirs = ["app", "src/app"]
    static let nextNames = ["icon.svg", "icon.png", "icon.jpg", "icon.ico", "apple-icon.png", "favicon.ico"]
    static let conventionalDirs = [
        "public", "static", "web", "web/icons", "assets", "static/img", "app/assets/images", "",
    ]
    static let conventionalNames = [
        "favicon.svg", "icon.svg", "apple-touch-icon.png", "icon.png", "favicon-96x96.png",
        "favicon-32x32.png", "android-chrome-192x192.png", "logo192.png", "Icon-192.png",
        "favicon.png", "favicon.ico",
    ]

    /// A declared icon and what its declaration says about its size, for ranking.
    struct Declared {
        var url: URL
        var sizes: String?
        var type: String?

        /// Vector first, then the smallest at least 48 px (the row's @2x size), then those
        /// of unknown size, then the largest of the smaller ones.
        var rank: (Int, Int) {
            let vector = sizes?.lowercased() == "any" || type?.lowercased() == "image/svg+xml"
                || url.pathExtension.lowercased() == "svg"
            if vector { return (0, 0) }
            guard let px = Self.pixels(sizes) else { return (2, 0) }
            return px >= 48 ? (1, px) : (3, -px)
        }

        /// The largest side in a `sizes` value such as "32x32 192x192".
        static func pixels(_ sizes: String?) -> Int? {
            sizes?.split(separator: " ").compactMap { size in
                size.lowercased().split(separator: "x").compactMap { Int($0) }.max()
            }.max()
        }
    }

    static func declared(in root: URL) -> [URL] {
        func ranked(_ icons: [Declared]) -> [URL] {
            icons.enumerated()
                .sorted { a, b in
                    (a.element.rank.0, a.element.rank.1, a.offset) < (b.element.rank.0, b.element.rank.1, b.offset)
                }
                .map(\.element.url)
        }
        var urls: [URL] = []
        for path in htmlFiles {
            let file = root.appendingPathComponent(path)
            guard let html = read(file, root: root) else { continue }
            urls += ranked(links(in: String(decoding: html, as: UTF8.self)).compactMap { link in
                resolve(link["href"], from: file.deletingLastPathComponent(), root: root).map {
                    Declared(url: $0, sizes: link["sizes"], type: link["type"])
                }
            })
        }
        for dir in manifestDirs {
            for name in manifestFiles {
                let file = root.appendingPathComponent(dir).appendingPathComponent(name)
                guard let data = read(file, root: root),
                      let manifest = try? JSONDecoder().decode(Manifest.self, from: data)
                else { continue }
                urls += ranked((manifest.icons ?? []).compactMap { icon in
                    resolve(icon.src, from: file.deletingLastPathComponent(), root: root).map {
                        Declared(url: $0, sizes: icon.sizes, type: icon.type)
                    }
                })
            }
        }
        for dir in nextDirs {
            urls += nextNames.map { root.appendingPathComponent(dir).appendingPathComponent($0) }
        }
        return urls
    }

    static func conventional(in root: URL) -> [URL] {
        conventionalDirs.flatMap { dir in
            conventionalNames.map { root.appendingPathComponent(dir).appendingPathComponent($0) }
        }
    }

    struct Manifest: Decodable {
        struct Icon: Decodable {
            var src: String
            var sizes: String?
            var type: String?
        }
        var icons: [Icon]?
    }

    private static let linkTag = try! NSRegularExpression(pattern: "<link\\b[^>]*>", options: .caseInsensitive)
    private static let attribute = try! NSRegularExpression(
        pattern: #"([a-zA-Z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#)

    /// The attributes of each `<link>` whose `rel` names an icon (`mask-icon` does not: it is
    /// a single-colour Safari pin).
    static func links(in html: String) -> [[String: String]] {
        let ns = html as NSString
        return linkTag.matches(in: html, range: NSRange(location: 0, length: ns.length)).compactMap { tag in
            let text = ns.substring(with: tag.range)
            var attributes: [String: String] = [:]
            for match in attribute.matches(in: text, range: NSRange(location: 0, length: (text as NSString).length)) {
                let name = (text as NSString).substring(with: match.range(at: 1)).lowercased()
                let value = (2...4).lazy.map { match.range(at: $0) }.first { $0.location != NSNotFound }
                    .map { (text as NSString).substring(with: $0) } ?? ""
                attributes[name] = attributes[name] ?? value
            }
            let rel = attributes["rel"]?.lowercased().split(separator: " ") ?? []
            return rel.contains("icon") || rel.contains("apple-touch-icon") ? attributes : nil
        }
    }

    /// A local path for an `href`: a leading `/` is tried under `public/`, `static/` and the
    /// root; anything else is relative to the declaring file. URLs with a scheme or host and
    /// `data:` URIs are skipped.
    static func resolve(_ href: String?, from base: URL, root: URL) -> URL? {
        guard var path = href?.trimmingCharacters(in: .whitespaces), !path.isEmpty else { return nil }
        let lower = path.lowercased()
        if lower.hasPrefix("//") || lower.hasPrefix("data:") || lower.contains("://") { return nil }
        if let end = path.firstIndex(where: { $0 == "?" || $0 == "#" }) { path = String(path[..<end]) }
        path = path.removingPercentEncoding ?? path
        guard !path.isEmpty else { return nil }
        if path.hasPrefix("/") {
            let relative = String(path.drop { $0 == "/" })
            return ["public", "static", ""].lazy
                .map { root.appendingPathComponent($0).appendingPathComponent(relative) }
                .first { FileManager.default.fileExists(atPath: $0.path) }
        }
        return base.appendingPathComponent(path)
    }

    // MARK: Checks

    /// `url` with symlinks resolved, if it is a regular file of at most 1 MB inside `root`.
    static func contained(_ url: URL, root: URL) -> URL? {
        let real = url.standardizedFileURL.resolvingSymlinksInPath()
        guard real.path.hasPrefix(root.path.hasSuffix("/") ? root.path : root.path + "/"),
              let values = try? real.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey]),
              values.isRegularFile == true, (values.fileSize ?? .max) <= maxBytes
        else { return nil }
        return real
    }

    static func read(_ url: URL, root: URL) -> Data? {
        contained(url, root: root).flatMap { try? Data(contentsOf: $0) }
    }

    /// The candidate as a `Found` if it is an SVG NSImage can draw, or a raster whose best
    /// image is at least 32 px on its short side; either way no wider than 3:2.
    static func check(_ url: URL, root: URL) -> Found? {
        guard let file = contained(url, root: root) else { return nil }
        if file.pathExtension.lowercased() == "svg" {
            guard let image = NSImage(contentsOf: file), image.isValid,
                  squareEnough(image.size.width, image.size.height)
            else { return nil }
            return Found(url: file, index: 0, vector: true)
        }
        guard let source = CGImageSourceCreateWithURL(file as CFURL, nil) else { return nil }
        let sizes = (0..<CGImageSourceGetCount(source)).compactMap { i -> (Int, Int, Int)? in
            guard let props = CGImageSourceCopyPropertiesAtIndex(source, i, nil) as? [CFString: Any],
                  let w = props[kCGImagePropertyPixelWidth] as? Int,
                  let h = props[kCGImagePropertyPixelHeight] as? Int
            else { return nil }
            return (i, w, h)
        }
        guard let best = sizes.max(by: { min($0.1, $0.2) < min($1.1, $1.2) }),
              min(best.1, best.2) >= minPixels, squareEnough(Double(best.1), Double(best.2))
        else { return nil }
        return Found(url: file, index: best.0, vector: false)
    }

    private static func squareEnough(_ w: Double, _ h: Double) -> Bool {
        w > 0 && h > 0 && max(w, h) / min(w, h) <= maxAspect
    }

    // MARK: Decoding

    /// The icon at most `pixels` on its long side. Rasters go through an ImageIO thumbnail,
    /// so a large PNG is never decoded whole.
    public static func image(_ found: Found, pixels: Int) -> CGImage? {
        if found.vector {
            guard let image = NSImage(contentsOf: found.url) else { return nil }
            let scale = Double(pixels) / max(image.size.width, image.size.height)
            var rect = CGRect(x: 0, y: 0, width: image.size.width * scale, height: image.size.height * scale)
            return image.cgImage(forProposedRect: &rect, context: nil, hints: nil)
        }
        guard let source = CGImageSourceCreateWithURL(found.url as CFURL, nil) else { return nil }
        return CGImageSourceCreateThumbnailAtIndex(source, found.index, [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: pixels,
        ] as CFDictionary)
    }

    // MARK: Revalidation

    /// Modification times of every directory searched, the declaring files and the icon,
    /// so adding, replacing or removing any of them changes the stamp.
    static func stamp(roots: [URL], found: Found?) -> [Double] {
        let dirs = Set(conventionalDirs + manifestDirs + nextDirs + ["src"])
        var files: [URL] = []
        for root in roots {
            files += dirs.sorted().map { root.appendingPathComponent($0) }
            files += htmlFiles.map { root.appendingPathComponent($0) }
            files += manifestDirs.flatMap { dir in
                manifestFiles.map { root.appendingPathComponent(dir).appendingPathComponent($0) }
            }
        }
        if let found { files.append(found.url) }
        return files.map {
            ((try? $0.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate)?
                .timeIntervalSince1970 ?? 0
        }
    }
}
