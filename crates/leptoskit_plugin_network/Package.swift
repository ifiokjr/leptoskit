// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitNetwork",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitNetwork", targets: ["LeptosKitNetwork"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitNetwork",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
