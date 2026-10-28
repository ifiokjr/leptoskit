// swift-tools-version:6.0
import PackageDescription

#if os(macOS)
let controllerTargets: [Target] = [
    .target(
        name: "LeptosKitMacControllers",
        dependencies: ["LeptosKitBridge"],
        path: ".",
        exclude: [
            "Cargo.toml", "apple", "apple-tests", "include", "harmonyos", "tests",
            "android", "benches", "android-tests", "async-host-tests",
            "apple-worker-tests", "src", "controller-tests", "ios-controls",
            "mobile/LeptosKitView.kt", "mobile/WidgetStyle.swift", "mobile/NativeReleases.kt",
            "mobile/WidgetTextStyle.swift", "mobile/WidgetImageStyle.swift",
            "mobile/NativeReleases.swift", "mobile/LeptosKitApplicationView.kt",
            "mobile/MobileWidgetRenderer.swift", "mobile/LeptosKitViewController.swift",
            "mobile/MobileApplicationChrome.swift", "desktop/LeptosKitMacViewController.swift",
            "mobile/NativeAppFactory.swift", "mobile/NativeAppRuntime.swift",
            "mobile/NativeRustWorker.kt", "mobile/NativeRustAsync.kt", "mobile/NativeApp.kt",
            "mobile/NativeRiveAnimator.kt", "mobile/NativeApplicationTeardown.kt", "mobile/NativePluginJobs.kt",
            "mobile/NativeServiceOwner.kt", "mobile/NativeTextMeasurer.kt",
            "mobile/NativeWidgetProps.kt", "mobile/NativeWidgetRenderer.kt",
            "mobile/LeptosKitApplicationController.swift", "mobile/app.cpp",
            "mobile/MobileApplicationTeardown.swift", "mobile/NativeDevelopmentGuests.swift",
            "mobile/NativeDevelopmentGuests.kt", "mobile/MobileApplicationDevelopment.swift",
            "mobile/MobileApplicationGuests.swift"
        ],
        sources: [
            "mobile/NativeRive.swift",
            "desktop/MacWidgetRenderer.swift", "desktop/MacTextLabel.swift", "mobile/RenderTransaction.swift",
            "mobile/WidgetProps.swift",
            "mobile/NativeRequestExecutor.swift", "mobile/NativeRustWorker.swift", "mobile/NativeRustAsync.swift"
        ]
    ),
    .testTarget(
        name: "LeptosKitMacControllerTests",
        dependencies: ["LeptosKitMacControllers"],
        path: "controller-tests"
    )
]
#else
let controllerTargets: [Target] = []
#endif

let package = Package(
    name: "LeptosKitBridge",
    platforms: [.macOS(.v14), .iOS(.v17)],
    products: [.library(name: "LeptosKitBridge", targets: ["LeptosKitBridge"])],
    targets: [
        .systemLibrary(name: "CLeptosKitBridge", path: "include"),
        .target(
            name: "LeptosKitBridge",
            dependencies: ["CLeptosKitBridge"],
            path: "apple",
            linkerSettings: [
                .linkedLibrary("leptoskit_bridge_ffi"),
                .linkedLibrary("c++", .when(platforms: [.macOS, .iOS]))
            ]
        ),
        .testTarget(
            name: "LeptosKitBridgeTests",
            dependencies: ["LeptosKitBridge", "WorkerTest"],
            path: "apple-tests"
        ),
        .target(
            name: "WorkerTest",
            path: "apple-worker-tests",
            exclude: ["WorkerLifecycle.swift", "workers.cpp"],
            sources: ["callbacks.cpp"],
            publicHeadersPath: "."
        )
    ] + controllerTargets,
    cxxLanguageStandard: .cxx17
)
