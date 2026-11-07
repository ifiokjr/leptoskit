// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBadge",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBadge", targets: ["LeptosKitBadge"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBadge",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitBadgeTests",
            dependencies: ["LeptosKitBadge"],
            path: "apple-tests"
        )
    ]
)
