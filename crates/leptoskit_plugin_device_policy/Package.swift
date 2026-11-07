// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitDevicePolicy",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitDevicePolicy", targets: ["LeptosKitDevicePolicy"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitDevicePolicy",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitDevicePolicyTests",
            dependencies: [
                "LeptosKitDevicePolicy",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
