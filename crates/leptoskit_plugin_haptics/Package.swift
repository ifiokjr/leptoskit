// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitHaptics",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitHaptics", targets: ["LeptosKitHaptics"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitHaptics",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
