//! Shipping source inventories shared by native project generators.

pub(crate) const MACOS: &[&str] = &[
	"desktop/LeptosKitMacViewController.swift",
	"desktop/MacWidgetRenderer.swift",
	"mobile/RenderTransaction.swift",
	"mobile/NativeRustWorker.swift",
	"mobile/NativeRustAsync.swift",
	"mobile/NativeAppFactory.swift",
	"mobile/NativeAppRuntime.swift",
	"mobile/NativeRequestExecutor.swift",
];

pub(crate) const IOS: &[&str] = &[
	"mobile/LeptosKitViewController.swift",
	"mobile/RenderTransaction.swift",
	"mobile/WidgetStyle.swift",
	"mobile/WidgetTextStyle.swift",
	"mobile/WidgetImageStyle.swift",
	"mobile/NativeReleases.swift",
	"mobile/LeptosKitApplicationController.swift",
	"mobile/NativeRustWorker.swift",
	"mobile/NativeRustAsync.swift",
	"mobile/NativeAppFactory.swift",
	"mobile/NativeAppRuntime.swift",
	"mobile/NativeRequestExecutor.swift",
	"mobile/MobileWidgetRenderer.swift",
	"mobile/MobileApplicationChrome.swift",
	"mobile/MobileApplicationTeardown.swift",
];

pub(crate) const ANDROID: &[&str] = &[
	"android/NativePermissionHost.kt",
	"android/NativeActivityResultHost.kt",
	"mobile/NativeApp.kt",
	"mobile/NativeRustWorker.kt",
	"mobile/NativeRustAsync.kt",
	"mobile/NativeReleases.kt",
	"mobile/LeptosKitApplicationView.kt",
	"mobile/NativeServiceOwner.kt",
	"mobile/NativePluginJobs.kt",
	"mobile/NativeWidgetRenderer.kt",
	"mobile/NativeWidgetProps.kt",
	"mobile/NativeTextMeasurer.kt",
	"mobile/NativeApplicationTeardown.kt",
];
