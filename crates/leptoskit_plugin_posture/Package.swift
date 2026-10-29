// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitPosture",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitPosture", targets: ["LeptosKitPosture"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitPosture",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
