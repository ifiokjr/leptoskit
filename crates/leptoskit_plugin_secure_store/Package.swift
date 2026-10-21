// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSecureStore",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSecureStore", targets: ["LeptosKitSecureStore"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitSecureStore",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitSecureStoreTests",
            dependencies: ["LeptosKitSecureStore"],
            path: "tests/apple"
        )
    ]
)
