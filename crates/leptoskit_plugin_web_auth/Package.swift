// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitWebAuth",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitWebAuth", targets: ["LeptosKitWebAuth"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitWebAuth",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitWebAuthTests",
            dependencies: [
                "LeptosKitWebAuth",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
