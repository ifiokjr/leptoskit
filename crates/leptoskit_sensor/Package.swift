// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSensors",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSensors", targets: ["LeptosKitSensors"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitSensors",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitSensorsTests",
            dependencies: ["LeptosKitSensors"],
            path: "apple-tests"
        )
    ]
)
