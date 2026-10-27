//! Shipping source inventories shared by native project generators.

pub(crate) const MACOS: &[&str] = &[
	"mobile/NativeRive.swift",
	"desktop/LeptosKitMacViewController.swift",
	"desktop/MacWidgetRenderer.swift",
	"mobile/RenderTransaction.swift",
	"mobile/NativeRustWorker.swift",
	"mobile/NativeRustAsync.swift",
	"mobile/NativeAppFactory.swift",
	"mobile/NativeAppRuntime.swift",
	"mobile/NativeRequestExecutor.swift",
	"mobile/NativeDevelopmentGuests.swift",
];

pub(crate) const IOS: &[&str] = &[
	"mobile/NativeRive.swift",
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
	"mobile/MobileApplicationDevelopment.swift",
	"mobile/MobileApplicationGuests.swift",
	"mobile/NativeDevelopmentGuests.swift",
];

pub(crate) const ANDROID: &[&str] = &[
	"mobile/NativeRiveAnimator.kt",
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
	"mobile/NativeDevelopmentGuests.kt",
];
