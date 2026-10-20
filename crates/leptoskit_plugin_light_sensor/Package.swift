// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLightSensor",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLightSensor", targets: ["LeptosKitLightSensor"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLightSensor",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(name: "LeptosKitLightSensorTests", dependencies: ["LeptosKitLightSensor"], path: "apple-tests")
    ]
)
