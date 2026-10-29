# Platforms

A leptoskit app is one tree of components. The web renders it as HTML through leptos. Every other platform renders it with that platform's own widgets, laid out by leptoskit's layout engine. This page lists the platforms, how to build for each, and what each one doesn't do yet.

| Platform  | Drawn with                          | Build command                        | Components in the app `leptoskit new` generates |
| --------- | ----------------------------------- | ------------------------------------ | ----------------------------------------------- |
| Web       | HTML and CSS, through leptos        | `leptoskit build --platform web`     | All                                             |
| Windows   | Win32 common controls               | `leptoskit build --platform windows` | All                                             |
| Linux     | GTK 4 widgets                       | `leptoskit build --platform linux`   | All                                             |
| macOS     | AppKit (`NSView`, `NSButton`, …)    | `leptoskit build --platform macos`   | Seven (see below)                               |
| iOS       | UIKit (`UIView`, `UIButton`, …)     | `leptoskit build --platform ios`     | Seven (see below)                               |
| Android   | Android views (`View`, `Button`, …) | `leptoskit build --platform android` | Seven (see below)                               |
| HarmonyOS | ArkUI components                    | `leptoskit native build harmonyos`   | Seven, experimental                             |
| Terminal  | Characters in the terminal's cells  | `leptoskit build --platform tui`     | All                                             |

macOS and iOS apps build on a Mac, the Windows app on Windows, and the Linux app on Linux. `leptoskit native build` takes the same platforms plus simulator and architecture variants: `ios-simulator`, `android-x86_64` for x86-64 emulators, and `harmonyos-x86_64`. [Host projects](host-projects.md) lists the tools each platform needs.

## Generated apps and backend crates

leptoskit draws native widgets in two places, and today they don't cover the same components.

**The apps `leptoskit new` generates** run your screens as WebAssembly, which is what lets [code push](../guide/05-code-push.md) update them. On Windows and Linux they use leptoskit's full Win32 and GTK backends, so every component works. On macOS, iOS, and Android they use a smaller Swift or Kotlin renderer that draws eight components: `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, `Image`, and `Vector`. Any other component, such as `Switch`, `Checkbox`, `Radio`, `Slider`, `PasswordInput`, `LinearProgressIndicator`, or `ActivityIndicator`, stops the app with an "unsupported widget" error. Of the accessibility props, they pass on only the accessible name (`aria_label`), not roles or states; a labelled `Vector` is announced as an image.

**The backend crates**, `leptoskit_appkit`, `leptoskit_uikit`, `leptoskit_android`, `leptoskit_gtk`, and `leptoskit_windows`, draw every built-in component. They are what the screenshots on this site, the [examples](../examples.md), and leptoskit's own widget tests use. You use them directly when you put a `NativeSurface` in your own native Rust app, for example with `UIKitHost` on iOS. Your screens are then compiled into the app rather than run as WebAssembly, so code push doesn't apply.

## What each platform doesn't do yet

The macOS backend crate is the reference. Compared with it:

| Platform  | Differences                                                                                                                                                                                                                                                                                                                |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| macOS     | The generated app draws eight components and doesn't report scroll events, which the backend crate does. Keyboard and VoiceOver activation don't show a button's pressed style.                                                                                                                                            |
| iOS       | The generated app draws eight components. In the backend, iOS has no native checkbox or radio button, so `Checkbox` and `Radio` are drawn with `UIButton`, and VoiceOver gets fewer roles than on macOS, with no expanded or collapsed state. Keyboard and VoiceOver activation don't show a button's pressed style.       |
| Android   | The generated app draws eight components. In the backend, a `Custom` widget kind with no registered native implementation stops the app instead of rendering an empty container, and TalkBack gets a smaller set of roles than macOS.                                                                                      |
| Linux     | Live regions update their text but aren't announced: announcements need GTK 4.14, and leptoskit supports GTK 4.6 and newer.                                                                                                                                                                                                |
| Windows   | `Switch` is drawn as a checkbox. Rounded corners are cut with a window region, so their edges aren't smoothed, and borders stay rectangular inside them. Accessibility has no expanded or collapsed state. A `Vector` with no background takes clicks only where it has ink.                                               |
| HarmonyOS | Only `View`, `Text`, `Button`, `TextInput`, `PasswordInput`, `ScrollView`, and `Image`; other components, including `Vector`, stop with an error. No code push, dark mode, right-to-left layout, horizontal scrolling, scroll events, native text measurement, or `active:` pressed styles. See [HarmonyOS](harmonyos.md). |
| Terminal  | The font is the user's terminal font: sizes and weights draw as attributes and colors, not different metrics. `Image` and `Vector` draw a placeholder texture, and right-to-left text is not reordered. See [Terminal](tui.md).                                                                                            |

Some limits apply to every native platform:

- `Image` loads local files, not URLs. iOS also accepts `resource://` names and Android `android.resource://` URIs.
- Shadows, transforms, gradients, custom fonts, line height, and letter spacing aren't supported yet. The [CSS coverage reference](../reference/css-coverage.md) lists what is.
- Links (`href`) only work on the web.
- The generated macOS, Windows, and Linux apps don't check for code push updates yet; only the iOS and Android apps do. Ship a new build to update a desktop app.

## How a native app is put together

```text
your app's screens, compiled to WebAssembly   ◀── replaced over the air by code push
        │ a tree of components
leptoskit renderer: diffs the tree, lays it out with flexbox and grid
        │ create, update, move, and remove widgets
platform backend: AppKit, UIKit, Android views, GTK 4, Win32, or ArkUI
        │
the platform's real widgets
```

Your screens run as WebAssembly inside a small interpreter that ships in the app. That is what lets [code push](../guide/05-code-push.md) replace them without an app store release: the interpreter, the renderer, and the widgets stay in the installed app, and only the WebAssembly changes. The interpreter is `wasmi`, which does not generate machine code, so it is allowed on iOS.

## Pages in this section

- [Code reuse](code-reuse.md): what you write once, what you write per platform, and where the seams are.
- [Web](web.md): server rendering, hydration, and serving.
- [iOS](ios.md), [Android](android.md), [macOS](macos.md), and [Windows and Linux](windows-linux.md): each platform's build, run, and release steps.
- [HarmonyOS](harmonyos.md): the experimental target and its limits.
- [Terminal](tui.md): the same screens in an interactive terminal, for long-lived apps.
- [Host projects](host-projects.md): the platform folders in your project, their requirements, and plugin registration.
- [Runtime plugins](plugins.md) and the [plugin support matrix](plugin-inventory.md): operating-system services such as the clipboard, location, and notifications.
