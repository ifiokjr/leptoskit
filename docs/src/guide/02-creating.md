# Creating a project

`leptoskit new` creates one app that runs in the browser and as a native app on iOS, Android, macOS, Windows, and Linux. You write the screens once; each platform draws them with its own widgets.

```bash
leptoskit new water
cd water
leptoskit dev
```

`leptoskit dev` builds the app, starts a server at `http://127.0.0.1:3000`, and rebuilds when you save. Open that address and you get a counter with an **Add one** button. The server renders the page as HTML first, and the browser then takes it over, so the button works. The same screen is what every native app shows: `dev` also opens it in the iOS Simulator, on an Android emulator, and as a desktop app, on every platform this machine can run, and updates them all when you save. [Building and serving](04-building.md#leptoskit-dev) lists what runs where.

## What's in the project

```text
water/
├── leptoskit.toml        app name, enabled platforms, build settings
├── Cargo.toml            the web server package and the workspace
├── src/main.rs           the web server: renders pages and mounts the backend
├── water_app/            your app: screens, components, state
│   ├── build.rs          turns on per-platform files (see below)
│   └── src/
│       ├── lib.rs        entry points for the browser, the server, and native apps
│       ├── routes.rs     declares the home screen module
│       ├── routes/home.rs  the home screen
│       ├── components/   reusable pieces of UI
│       └── services/     app state, such as the counter
├── water_backend/        server-only code, such as the `GET /api/health` endpoint
├── water_shared/         types the app and the backend both use
├── web/  assets/         the browser's HTML page and shared files
├── .github/workflows/release.yml
│                         publishes to the stores on a version tag, once you set them up
└── ios/  android/  macos/  windows/  linux/
                          one small native app per platform, which runs water_app
```

Most of your work happens in `water_app`. The platform folders hold what only that platform needs: its permissions, its startup code, and its native dependencies. Build output goes into `.leptoskit/`, which is ignored by Git.

## Make it yours

The starter counter is deliberately plain. Here is a screen that shows what leptoskit is for: one function, drawn by six different toolkits, that tells you which one is drawing it. Replace `water_app/src/routes/home.rs` with:

```rust
use leptoskit::prelude::*;

use crate::Counter;

/// Glasses of water in a day's goal.
const GOAL: i32 = 8;

/// Builds the same screen for the browser and every native app.
pub fn screen(state: &Counter) -> Element {
	let glasses = state.value();
	let filled = glasses.clamp(0, GOAL) as f32 / GOAL as f32;
	let counter = state.clone();

	view! {
		<View class="min-h-screen items-center justify-center gap-6 p-6 bg-sky-950 text-white">
			<Text class="text-3xl font-bold">"Water today"</Text>
			<Text class="text-xl">{format!("{glasses} of {GOAL} glasses")}</Text>
			{
				// A progress bar made of two boxes: the inner one is a
				// fraction of the outer one's width.
				view! {
					<View class="h-3 w-64 rounded-[6px] bg-sky-800">
						<View
							class="h-full rounded-[6px] bg-sky-400"
							with_style=move |style| style.width = Dimension::Percent(filled)
						/>
					</View>
				}
			}
			<Button
				class="rounded-lg bg-sky-400 px-6 py-3 font-semibold text-sky-950"
				on:press=move || counter.increment()
			>
				"Drink a glass"
			</Button>
			<Text class="text-sm text-sky-200">{format!("Drawn with {}", toolkit())}</Text>
		</View>
	}
}

/// The toolkit that draws this build. Each build turns on exactly one
/// platform feature, so only one branch is compiled in.
fn toolkit() -> &'static str {
	if cfg!(feature = "ios") {
		"UIKit"
	} else if cfg!(feature = "android") {
		"Android views"
	} else if cfg!(feature = "macos") {
		"AppKit"
	} else if cfg!(feature = "windows") {
		"Win32 controls"
	} else if cfg!(feature = "linux") {
		"GTK 4"
	} else {
		"the browser"
	}
}
```

Save, and the page reloads with a progress bar that fills as you tap. Here is what each part does:

- **`View`, `Text`, and `Button`** are leptoskit's built-in components. In the browser they become HTML elements. In a native app they become that platform's real controls: `Button` is a `UIButton` on iOS, an `NSButton` on macOS, an Android `Button`, a GTK button, or a Win32 `BUTTON`. Nothing runs in a web view.
- **`.class(...)`** takes Tailwind classes. They are built in: there is no Tailwind install and no CSS build step. In the browser they become CSS; on native, leptoskit's own layout engine applies them, so `items-center`, `gap-6`, and `w-64` lay the screen out the same way everywhere. `with_style` sets a style from a value instead, here the bar's width as a fraction of its track.
- **`Counter`** is the starter's state, in `water_app/src/services/`. `on_press` calls `increment`, and the screen is rebuilt from the new value: the browser re-renders it, and a native app commits only the widgets that changed.
- **`cfg!(feature = "ios")`** picks a value per platform. The CLI builds your app with one platform feature turned on: `web`, `ios`, `android`, `macos`, `windows`, or `linux`. Use the feature, not `target_os`: native apps run your code as WebAssembly, so `target_os` is the same on every native platform.

## Run it as a native app

Build the app for a platform with `--platform`:

```bash
leptoskit build --platform macos     # on a Mac: an .app in .leptoskit/macos/
leptoskit build --platform ios       # on a Mac: an iOS app for a device
leptoskit native build ios-simulator # on a Mac: an iOS app for the simulator
leptoskit build --platform android   # an arm64 APK
leptoskit build --platform windows   # on Windows: a Win32 executable
leptoskit build --platform linux     # on Linux: a GTK 4 executable
```

Each build prints where it wrote the app. Apple apps build on a Mac, the Windows app on Windows, and the Linux app on Linux; Android builds anywhere its SDK runs. A native build needs that platform's tools on your machine: Xcode for iOS and macOS, the Android SDK and NDK for Android, the MSVC toolchain on Windows, and GTK 4 on Linux. You can create the project without any of them. [Host projects](../platforms/host-projects.md) lists each platform's requirements and how to open the project in Xcode or Android Studio.

On macOS, open the app:

```bash
open .leptoskit/macos/App.app
```

The same screen appears in a native window, and the last line reads **Drawn with AppKit**.

<Callout kind="important" title="Components in the generated apps">

The macOS, iOS, and Android apps that `leptoskit new` generates draw `View`, `Text`, `Button`, `TextInput`, `ScrollView`, and `Image`. Any other component, such as `Switch`, `Slider`, or `PasswordInput`, stops those apps with an "unsupported widget" error for now. The Windows and Linux apps, and the web, draw every component. [Platforms](../platforms/readme.md#what-each-platform-doesnt-do-yet) has the details.

</Callout>

The CLI picks the browser when you don't pass `--platform`. To make another platform the default, create the project with it, for example `leptoskit new water --platform ios`, or change `app.default_platform` in `leptoskit.toml`. Every platform folder is generated either way. `leptoskit new .` creates the project in the current, empty directory, and `--name <NAME>` sets the app name when it differs from the directory. `leptoskit new` never overwrites existing files.

HarmonyOS is not part of this project layout yet. It has its own experimental starter: see [HarmonyOS](../platforms/harmonyos.md).

## Use a different file on one platform

Sometimes one screen should differ on one platform. Say your iOS app should use iOS's larger title style, while every other platform keeps the shared screen. Rather than filling the screen with `cfg!` checks, give iOS its own copy of the file.

The starter is already set up for this. `water_app/src/routes.rs` declares the home module with `platform_module!` instead of a plain `mod home;`:

```rust
leptoskit::platform_module!(pub mod home = "routes/home");
```

`"routes/home"` is the module's path inside `water_app/src`, without the `.rs`. Next to the shared file, add one named for the platform:

```text
water_app/src/routes/
├── home.rs       used by web, Android, macOS, Windows, and Linux
└── home.ios.rs   used by iOS builds only
```

The starter calls `screen` directly, so this entry point keeps its name; reusable components should use `#[component]` and PascalCase.

`home.ios.rs` must provide everything callers use from `home.rs`. Here, that is `pub fn screen(state: &Counter) -> Element`:

```rust
use leptoskit::prelude::*;

use crate::Counter;

pub fn screen(state: &Counter) -> Element {
	let counter = state.clone();

	view! {
		<View class="min-h-screen gap-4 px-4 pt-safe-offset-4 bg-white">
			<Text class="text-3xl font-bold text-slate-900">"Water"</Text>
			<Text class="text-lg text-slate-500">
				{format!("{} glasses today", state.value())}
			</Text>
			<Button on:press=move || counter.increment()>"Drink a glass"</Button>
		</View>
	}
}
```

Now `leptoskit build --platform ios` compiles `home.ios.rs`, and every other platform compiles `home.rs`. Code elsewhere keeps calling `crate::routes::home::screen`; it never knows which file it got. The suffixes are `.web.rs`, `.ios.rs`, `.android.rs`, `.macos.rs`, `.windows.rs`, and `.linux.rs`.

Two pieces make this work. `water_app/build.rs` calls `leptoskit_build::platforms()`, which reads the platform feature the CLI turned on and watches `src/`, so adding or removing an override file triggers a rebuild. And `platform_module!` picks the file for that platform, falling back to the shared one. A module declared with a plain `mod` never looks for overrides, so use `platform_module!` for each module that needs them.

## Add a dependency for one platform

The same features control dependencies. To use a crate only on iOS, make it optional and add it to the `ios` feature in `water_app/Cargo.toml`:

```toml
[dependencies]
serde_json = { version = "1", optional = true }

[features]
ios = ["native", "leptoskit/ios", "dep:serde_json"]
```

Keep the entries the starter put in each feature; add yours after them. Your app compiles to WebAssembly on every native platform, so a dependency added here must build for `wasm32-unknown-unknown`. Crates that call UIKit, Android's Java APIs, or other operating system libraries can't. Put those in the platform's folder, such as `ios/Cargo.toml`, or use a [runtime plugin](../platforms/plugins.md).
