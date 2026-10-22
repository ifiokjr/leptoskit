// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitScreenCapture",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitScreenCapture", targets: ["LeptosKitScreenCapture"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitScreenCapture",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitScreenCaptureTests",
            dependencies: [
                "LeptosKitScreenCapture",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
