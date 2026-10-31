// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitPasskeys",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitPasskeys", targets: ["LeptosKitPasskeys"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitPasskeys",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitPasskeysTests",
            dependencies: [
                "LeptosKitPasskeys",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
