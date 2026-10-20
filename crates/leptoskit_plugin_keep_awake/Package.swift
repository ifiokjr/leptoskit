// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitKeepAwake",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitKeepAwake", targets: ["LeptosKitKeepAwake"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitKeepAwake",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitKeepAwakeTests",
            dependencies: ["LeptosKitKeepAwake"],
            path: "apple-tests"
        )
    ]
)
