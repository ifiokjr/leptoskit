// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLiveActivities",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLiveActivities", targets: ["LeptosKitLiveActivities"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitLiveActivities", dependencies: [
            .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
        ], path: "apple"),
        .testTarget(
            name: "LiveActivitiesTests",
            dependencies: ["LeptosKitLiveActivities"],
            path: "apple-tests"
        )
    ]
)
