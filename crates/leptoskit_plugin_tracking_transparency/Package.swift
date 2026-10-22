// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitTrackingTransparency",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitTrackingTransparency", targets: ["LeptosKitTrackingTransparency"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitTrackingTransparency", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(
            name: "TrackingTransparencyTests",
            dependencies: ["LeptosKitTrackingTransparency"],
            path: "apple-tests"
        )
    ]
)
