// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitHomeWidgets",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitHomeWidgets", targets: ["LeptosKitHomeWidgets"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitHomeWidgets",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitHomeWidgetsTests",
            dependencies: ["LeptosKitHomeWidgets"],
            path: "apple-tests"
        )
    ]
)
