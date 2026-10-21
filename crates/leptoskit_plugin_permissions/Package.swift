// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitPermissions",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitPermissions", targets: ["LeptosKitPermissions"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitPermissions",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
