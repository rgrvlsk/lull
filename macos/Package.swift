// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Lull",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "LullKit", targets: ["LullKit"]),
        .executable(name: "LullApp", targets: ["LullApp"]),
    ],
    targets: [
        // Socket client, Codable models and the observable store (WP4.1).
        .target(name: "LullKit", path: "Sources/LullKit"),
        // Menu-bar app stub; the real app arrives with WP4.2–4.4.
        .executableTarget(name: "LullApp", dependencies: ["LullKit"], path: "Sources/LullApp"),
        .testTarget(name: "LullKitTests", dependencies: ["LullKit"], path: "Tests/LullKitTests"),
    ],
    swiftLanguageModes: [.v6]
)
