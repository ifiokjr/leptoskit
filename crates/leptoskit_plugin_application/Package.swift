// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitApplication",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitApplication", targets: ["LeptosKitApplication"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitApplication",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
