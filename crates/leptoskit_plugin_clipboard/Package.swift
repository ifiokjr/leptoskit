// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitClipboard",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitClipboard", targets: ["LeptosKitClipboard"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitClipboard",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitClipboardTests",
            dependencies: ["LeptosKitClipboard"],
            path: "apple-tests"
        )
    ]
)
