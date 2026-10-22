// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitLocalAuthentication",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitLocalAuthentication", targets: ["LeptosKitLocalAuthentication"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitLocalAuthentication",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitLocalAuthenticationTests",
            dependencies: [
                "LeptosKitLocalAuthentication",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
