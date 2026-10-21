// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLocalization",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLocalization", targets: ["LeptosKitLocalization"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLocalization",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
