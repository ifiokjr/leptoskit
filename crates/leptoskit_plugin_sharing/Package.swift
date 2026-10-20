// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSharing",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSharing", targets: ["LeptosKitSharing"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitSharing",
                dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")], path: "apple"),
        .testTarget(name: "SharingTests", dependencies: ["LeptosKitSharing"], path: "apple-tests")
    ]
)
