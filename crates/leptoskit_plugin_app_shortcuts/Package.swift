// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitAppShortcuts",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitAppShortcuts", targets: ["LeptosKitAppShortcuts"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "LeptosKitAppShortcuts",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        ),
        .testTarget(
            name: "LeptosKitAppShortcutsTests",
            dependencies: ["LeptosKitAppShortcuts"],
            path: "apple-tests"
        )
    ]
)
