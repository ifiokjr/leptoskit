// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitNotifications",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitNotifications", targets: ["LeptosKitNotifications"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitNotifications",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitNotificationsTests",
            dependencies: ["LeptosKitNotifications"],
            path: "apple-tests"
        )
    ]
)
