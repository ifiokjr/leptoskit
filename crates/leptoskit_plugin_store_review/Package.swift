// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitStoreReview",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitStoreReview", targets: ["LeptosKitStoreReview"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitStoreReview",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitStoreReviewTests",
            dependencies: ["LeptosKitStoreReview"],
            path: "apple-tests"
        )
    ]
)
