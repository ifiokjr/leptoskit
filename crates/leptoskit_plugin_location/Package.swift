// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLocation",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLocation", targets: ["LeptosKitLocation"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLocation",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(name: "LeptosKitLocationTests", dependencies: ["LeptosKitLocation"], path: "apple-tests")
    ]
)
