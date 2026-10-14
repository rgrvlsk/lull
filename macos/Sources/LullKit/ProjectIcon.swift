// A server's project icon, read from the project's files (contract §10). Never fetched from
// the server: lull makes no network requests (§14), and a request would count as traffic.
//
// Search roots are the server's cwd, then its git root. In each, the first candidate that
// passes the size check wins: icons the project declares (`<link rel=icon>` in HTML entry
// points and layout templates, web manifests, Next.js `app/` files), then conventional
// file names in conventional directories. When no root has one, the first small icon
// (16–31 px, usually an old favicon) is used instead, drawn at its own size rather than
// scaled up. File contents are untrusted: only ImageIO, NSImage (for SVGs that refer to
// nothing outside themselves) and a minimal attribute scan read them, and no path may
// leave its search root.
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
        /// Under `minPixels` on its short side: drawn at its own size, never scaled up.
        public var small = false
    }

    /// What `lookup` returns: the decoded icon, if any, and the modification times that
    /// decide whether it needs looking up again.
    public struct Lookup: @unchecked Sendable {
        public var image: CGImage?
        public var found: Found?
        public var stamp: [Double]
        /// A small icon's average colour, for the tile behind it.
        public var tint: CGColor?
    }

    static let maxBytes = 1 << 20
    static let minPixels = 32
    static let minSmallPixels = 16
    static let maxAspect = 1.5

    // MARK: Lookup

    /// Finds and decodes the icon for a server in `cwd`, at most `pixels` on its long side.
    public static func lookup(cwd: String, pixels: Int) -> Lookup {
        let roots = searchRoots(cwd: cwd)
        let candidates = roots.compactMap(find(in:))
        let found = candidates.first { !$0.small } ?? candidates.first
        let decoded = found.flatMap { image($0, pixels: pixels) }
        return Lookup(image: decoded, found: found, stamp: stamp(roots: roots, found: found),
                      tint: found?.small == true ? decoded.flatMap(averageColor) : nil)
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

    /// The first candidate under `root` that passes the size check, else the first small one.
    public static func find(in root: URL) -> Found? {
        var small: Found?
        for candidate in declared(in: root) + conventional(in: root) {
            guard let found = check(candidate, root: root) else { continue }
            if !found.small { return found }
            small = small ?? found
        }
        return small
    }

    // MARK: Candidates

    /// Where pages and layout templates usually live, from plain `index.html` to Blade,
    /// Astro, SvelteKit, Next, Rails and Django, each listed one level deep.
    static let templateDirs = [
        "", "public", "src", "src/app", "app", "src/routes", "pages", "src/pages", "src/layouts",
        "src/components", "resources/views", "resources/views/layouts",
        "resources/views/components/layouts", "app/views/layouts", "templates",
    ]
    static let templateExtensions: Set = [
        "html", "htm", "astro", "php", "erb", "twig", "jinja", "jinja2", "j2", "njk", "hbs",
        "liquid", "heex", "svelte", "vue", "tsx", "jsx",
    ]
    /// Only files named like a page shell are read, so component folders cost little.
    static let templateHints = [
        "index", "app", "layout", "root", "base", "head", "document", "main", "master", "welcome",
    ]
    static let manifestFiles = ["manifest.json", "site.webmanifest", "manifest.webmanifest"]
    static let manifestDirs = ["", "public", "static", "app"]
    static let nextDirs = ["app", "src/app"]
    static let nextNames = ["icon.svg", "icon.png", "icon.jpg", "icon.ico", "apple-icon.png", "favicon.ico"]
    static let conventionalDirs = [
        "public", "static", "web", "web/icons", "assets", "static/img", "app/assets/images",
        "public/images", "public/img", "src", "src/assets", "priv/static", "src-tauri/icons", "",
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
        for file in templateFiles(in: root) {
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

    /// The page and layout templates under `root`, in `templateDirs` order and by name within
    /// each directory.
    static func templateFiles(in root: URL) -> [URL] {
        templateDirs.flatMap { dir -> [URL] in
            let names = (try? FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent(dir).path)) ?? []
            return names.sorted().compactMap { name in
                let lower = name.lowercased()
                let url = root.appendingPathComponent(dir).appendingPathComponent(name)
                guard templateExtensions.contains(url.pathExtension.lowercased()),
                      templateHints.contains(where: lower.contains)
                else { return nil }
                return url
            }
        }
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
        pattern: #"([a-zA-Z-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|\{([^}]*)\}|([^\s"'>]+))"#)

    /// The attributes of each `<link>` whose `rel` names an icon (`mask-icon` does not: it is
    /// a single-colour Safari pin).
    static func links(in html: String) -> [[String: String]] {
        let ns = html as NSString
        return linkTag.matches(in: html, range: NSRange(location: 0, length: ns.length)).compactMap { tag in
            let text = ns.substring(with: tag.range)
            var attributes: [String: String] = [:]
            for match in attribute.matches(in: text, range: NSRange(location: 0, length: (text as NSString).length)) {
                let name = (text as NSString).substring(with: match.range(at: 1)).lowercased()
                let value = (2...5).lazy.map { match.range(at: $0) }.first { $0.location != NSNotFound }
                    .map { (text as NSString).substring(with: $0) } ?? ""
                attributes[name] = attributes[name] ?? value
            }
            let rel = attributes["rel"]?.lowercased().split(separator: " ") ?? []
            return rel.contains("icon") || rel.contains("apple-touch-icon") ? attributes : nil
        }
    }

    private static let templatePath = try! NSRegularExpression(
        pattern: #"[A-Za-z0-9_@./~-]*\.(?:svg|png|ico|jpe?g|webp|gif|avif)\b"#, options: .caseInsensitive)

    /// A local path for an `href`. One a template computes, such as `{{ asset('logo.png') }}`,
    /// `{% static 'logo.png' %}` or `%sveltekit.assets%/favicon.png`, is reduced to its image
    /// path and treated as site-absolute. A site-absolute path is tried under `public/`,
    /// `static/` and the root; anything else relative to the declaring file first. URLs with
    /// a scheme or host and `data:` URIs are skipped.
    static func resolve(_ href: String?, from base: URL, root: URL) -> URL? {
        guard var path = href?.trimmingCharacters(in: .whitespaces), !path.isEmpty else { return nil }
        let lower = path.lowercased()
        if lower.hasPrefix("//") || lower.hasPrefix("data:") || lower.contains("://") { return nil }
        let computed = ["{", "<%", "%sveltekit", "("].contains(where: path.contains)
        if computed {
            let ns = path as NSString
            guard let match = templatePath.firstMatch(in: path, range: NSRange(location: 0, length: ns.length))
            else { return nil }
            path = "/" + ns.substring(with: match.range).drop { "~@/".contains($0) }
        }
        if let end = path.firstIndex(where: { $0 == "?" || $0 == "#" }) { path = String(path[..<end]) }
        path = path.removingPercentEncoding ?? path
        guard !path.isEmpty else { return nil }
        let exists = { (url: URL) in FileManager.default.fileExists(atPath: url.path) }
        if !path.hasPrefix("/") {
            // Relative to the page, which for a server template is its URL, not its file.
            let beside = base.appendingPathComponent(path)
            if exists(beside) { return beside }
        }
        let relative = String(path.drop { $0 == "/" })
        return ["public", "static", ""].lazy
            .map { root.appendingPathComponent($0).appendingPathComponent(relative) }
            .first(where: exists)
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

    /// The candidate as a `Found` if it is a self-contained SVG NSImage can draw, or a raster
    /// whose best image is at least 16 px on its short side (small under 32); either way no
    /// wider than 3:2.
    static func check(_ url: URL, root: URL) -> Found? {
        guard let file = contained(url, root: root) else { return nil }
        if file.pathExtension.lowercased() == "svg" {
            guard let data = read(file, root: root), selfContained(svg: data),
                  let image = NSImage(data: data), image.isValid,
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
              min(best.1, best.2) >= minSmallPixels, squareEnough(Double(best.1), Double(best.2))
        else { return nil }
        return Found(url: file, index: best.0, vector: false, small: min(best.1, best.2) < minPixels)
    }

    private static let outsideReference = try! NSRegularExpression(
        pattern: #"(?:href|src)\s*=\s*["']\s*(?!#|data:)|url\(\s*["']?\s*(?!#|data:)|@import|<!ENTITY"#,
        options: .caseInsensitive)

    /// Whether an SVG refers to nothing outside itself. One that embeds another file or URL
    /// (an `<image>` wrapping the real logo, a `<use>` of another file, a stylesheet) draws a
    /// broken-image placeholder, and lull never loads what it points at.
    static func selfContained(svg: Data) -> Bool {
        let text = String(decoding: svg, as: UTF8.self)
        return outsideReference.firstMatch(in: text, range: NSRange(location: 0, length: (text as NSString).length)) == nil
    }

    private static func squareEnough(_ w: Double, _ h: Double) -> Bool {
        w > 0 && h > 0 && max(w, h) / min(w, h) <= maxAspect
    }

    // MARK: Decoding

    /// The icon at most `pixels` on its long side, never larger than the file's own image.
    /// Rasters go through an ImageIO thumbnail, so a large PNG is never decoded whole.
    public static func image(_ found: Found, pixels: Int) -> CGImage? {
        if found.vector {
            guard let data = try? Data(contentsOf: found.url), selfContained(svg: data),
                  let image = NSImage(data: data)
            else { return nil }
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

    /// The image's average colour, ignoring transparent pixels.
    static func averageColor(_ image: CGImage) -> CGColor? {
        var pixel = [UInt8](repeating: 0, count: 4)
        let drawn = pixel.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(
                data: buffer.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return false }
            context.interpolationQuality = .medium
            context.draw(image, in: CGRect(x: 0, y: 0, width: 1, height: 1))
            return true
        }
        let alpha = Double(pixel[3])
        guard drawn, alpha > 8 else { return nil }
        return CGColor(srgbRed: Double(pixel[0]) / alpha, green: Double(pixel[1]) / alpha,
                       blue: Double(pixel[2]) / alpha, alpha: 1)
    }

    // MARK: Revalidation

    /// Modification times of every directory searched, the declaring files and the icon,
    /// so adding, replacing or removing any of them changes the stamp.
    static func stamp(roots: [URL], found: Found?) -> [Double] {
        let dirs = Set(conventionalDirs + manifestDirs + nextDirs + templateDirs)
        var files: [URL] = []
        for root in roots {
            files += dirs.sorted().map { root.appendingPathComponent($0) }
            files += templateFiles(in: root)
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
