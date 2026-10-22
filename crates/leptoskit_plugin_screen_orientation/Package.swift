// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitScreenOrientation",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitScreenOrientation", targets: ["LeptosKitScreenOrientation"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitScreenOrientation", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(name: "ScreenOrientationTests", dependencies: ["LeptosKitScreenOrientation"], path: "apple-tests")
    ]
)
