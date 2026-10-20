// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBattery",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBattery", targets: ["LeptosKitBattery"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitBattery",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitBatteryTests",
            dependencies: ["LeptosKitBattery", .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple-tests"
        )
    ]
)
