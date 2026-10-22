// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitPedometer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitPedometer", targets: ["LeptosKitPedometer"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitPedometer",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(name: "LeptosKitPedometerTests", dependencies: ["LeptosKitPedometer"], path: "apple-tests")
    ]
)
