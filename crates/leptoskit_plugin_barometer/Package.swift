// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitBarometer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBarometer", targets: ["LeptosKitBarometer"])],
    dependencies: [.package(path: "../leptoskit_sensor")],
    targets: [
        .target(
            name: "LeptosKitBarometer",
            dependencies: [.product(name: "LeptosKitSensors", package: "leptoskit_sensor")],
            path: "apple"
        )
    ]
)
