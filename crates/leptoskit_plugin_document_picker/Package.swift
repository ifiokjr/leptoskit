// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "LeptosKitDocumentPicker",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitDocumentPicker", targets: ["LeptosKitDocumentPicker"])],
    dependencies: [.package(path: "../leptoskit_bridge_ffi")],
    targets: [
        .target(name: "LeptosKitDocumentPicker",
                dependencies: [.product(name: "LeptosKitBridge", package: "leptoskit_bridge_ffi")], path: "apple"),
        .testTarget(name: "DocumentPickerTests", dependencies: ["LeptosKitDocumentPicker"], path: "apple-tests")
    ]
)
