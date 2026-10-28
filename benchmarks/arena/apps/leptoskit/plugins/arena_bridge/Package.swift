// swift-tools-version:6.0
import PackageDescription

// The CLI copies this package beside its own copy of leptoskit_bridge_ffi.
let package = Package(
    name: "ArenaBridge",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "ArenaBridge", targets: ["ArenaBridge"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(
            name: "ArenaBridge",
            dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")],
            path: "apple"
        )
    ]
)
