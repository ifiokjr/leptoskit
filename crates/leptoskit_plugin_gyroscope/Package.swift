// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitGyroscope",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitGyroscope", targets: ["LeptosKitGyroscope"])],
    dependencies: [.package(path: "../leptoskit_sensor")],
    targets: [
        .target(
            name: "LeptosKitGyroscope",
            dependencies: [.product(name: "LeptosKitSensors", package: "leptoskit_sensor")],
            path: "apple"
        )
    ]
)
