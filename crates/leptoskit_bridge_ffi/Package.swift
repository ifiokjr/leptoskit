// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBridge",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBridge", targets: ["LeptosKitBridge"])],
    targets: [
        .systemLibrary(name: "CLeptosKitBridge", path: "include"),
        .target(
            name: "LeptosKitBridge",
            dependencies: ["CLeptosKitBridge"],
            path: "apple",
            linkerSettings: [.linkedLibrary("leptoskit_bridge_ffi")]
        ),
        .testTarget(
            name: "LeptosKitBridgeTests",
            dependencies: ["LeptosKitBridge"],
            path: "apple-tests"
        )
    ]
)
