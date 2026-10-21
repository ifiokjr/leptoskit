// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSpeech",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSpeech", targets: ["LeptosKitSpeech"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitSpeech",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitSpeechTests",
            dependencies: [
                "LeptosKitSpeech",
                .product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")
            ],
            path: "apple-tests"
        )
    ]
)
