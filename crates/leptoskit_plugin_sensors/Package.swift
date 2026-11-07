// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitSensorsPlugin",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitSensorsPlugin", targets: ["LeptosKitSensorsPlugin"])],
    dependencies: [.package(path: "../leptoskit_sensor")],
    targets: [
        .target(
            name: "LeptosKitSensorsPlugin",
            dependencies: [.product(name: "LeptosKitSensors", package: "leptoskit_sensor")],
            path: "apple"
        )
    ]
)
