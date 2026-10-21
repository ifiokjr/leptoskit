// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitAccelerometer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitAccelerometer", targets: ["LeptosKitAccelerometer"])],
    dependencies: [.package(path: "../leptoskit_sensor")],
    targets: [
        .target(
            name: "LeptosKitAccelerometer",
            dependencies: [.product(name: "LeptosKitSensors", package: "leptoskit_sensor")],
            path: "apple"
        )
    ]
)
