// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLinking",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLinking", targets: ["LeptosKitLinking"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLinking",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
