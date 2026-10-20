// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBrightness",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBrightness", targets: ["LeptosKitBrightness"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBrightness",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
