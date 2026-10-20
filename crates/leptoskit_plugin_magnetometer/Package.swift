// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitMagnetometer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitMagnetometer", targets: ["LeptosKitMagnetometer"])],
    dependencies: [.package(path: "../leptoskit_sensor")],
    targets: [
        .target(
            name: "LeptosKitMagnetometer",
            dependencies: [.product(name: "LeptosKitSensors", package: "leptoskit_sensor")],
            path: "apple"
        )
    ]
)
