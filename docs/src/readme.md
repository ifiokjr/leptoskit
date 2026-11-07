<Hero start="guide/01-installation.md">

# One leptos tree. Real native widgets.

leptoskit is a Rust toolkit for building an app's interface once, with leptos components and built-in Tailwind styles, and shipping it to the browser and as real native widgets on the desktop and on phones.

</Hero>

## What leptoskit is for

Most cross-platform toolkits make you pick: render to a web view everywhere, or write each platform twice. leptoskit takes a third path.

You describe a screen once, in Rust, as a tree of components: `View`, `Text`, `Button`, `TextInput`, `ScrollView`, and the rest. On the web, the same tree renders through leptos to the DOM, with server rendering and hydration. On native platforms, leptoskit's own layout engine lays the tree out with flexbox and CSS grid, and a platform backend commits it as the platform's real controls: AppKit views on macOS, UIKit on iOS, Android views, GTK 4 on Linux, Win32 controls on Windows, and, experimentally, ArkUI on HarmonyOS. In a terminal, the same tree draws as characters in the cell grid. Nothing runs in a web view.

<Cards>
	<Card title="Get started" href="guide/01-installation.md">
		Install the CLI, scaffold a project, and see it running in a browser in a few minutes.
	</Card>
	<Card title="Build a shared screen" href="guide/07-shared-components.md">
		Write one screen with the built-in components and Tailwind, and render it on the web and natively.
	</Card>
	<Card title="Test what you ship" href="guide/08-testing.md">
		Widget tests, device journeys, and the same finders on every platform.
	</Card>
	<Card title="Pick your platforms" href="platforms/readme.md">
		What each target runs, what works today, and how the host apps are put together.
	</Card>
</Cards>

## A screen in leptoskit

A screen is a function returning an `Element`. Tailwind classes style it on every platform:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Welcome(name: &str, on_start: impl Fn() + Send + Sync + 'static) -> Element {
	view! {
		<View class="gap-4 rounded-2xl bg-white p-6 dark:bg-slate-900">
			<Text class="text-2xl font-bold">{format!("Hello, {name}")}</Text>
			<Text class="text-slate-500">"Your interface, everywhere."</Text>
			<Button class="rounded-lg bg-rose-500 px-4 py-2 text-white" on:press=on_start>
				"Start"
			</Button>
		</View>
	}
}
```

The same function renders as DOM elements in a browser, and through a platform backend as `NSView`s, `UIView`s, Android views, GTK widgets, Win32 controls, or ArkUI components. [The shared components guide](guide/07-shared-components.md) walks through it, and the [component catalog](reference/components.md) lists every component.

## See it render

These screens are leptoskit element trees committed as real AppKit views and rendered offscreen. They are not mockups. [See all of them](examples.md).

<Gallery>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen rendered with native AppKit views" title="Profile">An avatar over the header and wrapping text measured with AppKit's font metrics.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen rendered with native AppKit views" title="Counter">A centred card, a progress bar of nested flex boxes, and a pill button.</Shot>
	<Shot src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat screen rendered with native AppKit views" title="Chat">Bubbles capped with `max_width`, aligned by `align_self`.</Shot>
</Gallery>

## How the pieces fit

The logo is the architecture. The ember boxes are layout: elements placed by the layout engine and committed as native widgets. The mint piece is the next version of your interface arriving over the air and dropping into place. Hover over it at the top of this page.

| Crate                                                                                              | What it does                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`leptoskit`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit)                   | The crate your app depends on. It re-exports the components, styles, and runtime, with a feature per optional part.                                                                                                                                                                                                                                                                                                                                |
| [`leptoskit_cli`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_cli)           | The `leptoskit` command, also installed as `lk`, which scaffolds, builds, serves, tests, formats, and ships apps.                                                                                                                                                                                                                                                                                                                                  |
| [`leptoskit_native`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_native)     | The components, Tailwind support, and the renderer that diffs element trees into mutation batches for a platform backend.                                                                                                                                                                                                                                                                                                                          |
| [`leptoskit_yoga`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_yoga)         | The layout engine behind native rendering: flexbox and CSS grid, gaps, aspect ratio, and right-to-left layout.                                                                                                                                                                                                                                                                                                                                     |
| [`leptoskit_appkit`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_appkit)     | The macOS backend. [`leptoskit_uikit`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_uikit), [`leptoskit_android`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_android), [`leptoskit_gtk`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_gtk), and [`leptoskit_windows`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_windows) do the same for their platforms. |
| [`leptoskit_wasm`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_wasm)         | The on-device runtime. It runs the interface bundle in `wasmi`, an interpreter that needs no JIT.                                                                                                                                                                                                                                                                                                                                                  |
| [`leptoskit_codepush`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_codepush) | Over-the-air updates: signed, content-hashed releases, binary deltas, and a client that verifies everything before it stages.                                                                                                                                                                                                                                                                                                                      |
| [`leptoskit_axum`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_axum)         | The server toolkit: static files with precompressed copies, SPA fallback, health checks, and the code push registry, on Axum.                                                                                                                                                                                                                                                                                                                      |
| [`leptoskit_router`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_router)     | File-based routing. `src/routes/` becomes one typed `Route` enum.                                                                                                                                                                                                                                                                                                                                                                                  |
| [`leptoskit_query`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_query)       | Cached, deduplicated, retrying async data in the style of TanStack Query.                                                                                                                                                                                                                                                                                                                                                                          |
| [`leptoskit_l10n`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_l10n)         | Typed translations in the style of Flutter's `gen_l10n`: ARB files become a `Messages` trait, with plurals for every language.                                                                                                                                                                                                                                                                                                                     |
| [`leptoskit_testing`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_testing)   | Widget tests with semantic finders, plus a driver that runs the same journeys against real native apps.                                                                                                                                                                                                                                                                                                                                            |
| [`leptoskit_docs`](https://github.com/leptoskit/leptoskit/tree/main/crates/leptoskit_docs)         | Markdown with leptos components, and the generator that builds this site. See [writing docs](guide/14-writing-docs.md).                                                                                                                                                                                                                                                                                                                            |

## Over the air updates

An installed native app is a small host that runs your interface as a WebAssembly bundle. Code push ships a new bundle over the air: signed, content-hashed releases, binary deltas, and a client that verifies every byte before it stages the update. A trial release replaces the working one only after it renders, so a release that fails to start is rejected and the app keeps running. The generated iOS and Android apps install these releases today; the generated macOS, Windows, and Linux apps don't check for updates yet. A release must match the installed host's interface contract and signing policy, and store rules still apply to what a download changes. [Code push](guide/05-code-push.md) covers keys, channels, publishing, and rollback.

## Where things stand

leptoskit is young, and these docs say plainly what works today.

Native filesystem, SQLite, and secure-store plugins enforce aggregate storage budgets in addition to operation limits. See [storage and networking](platforms/plugin-inventory.md#storage-and-networking) for defaults and upgrade behavior.

<Callout kind="note" title="Works now">

The web pipeline: cached builds, `wasm-opt`, server rendering, islands, and Axum serving. The layout engine and the renderer. Native backends for macOS, iOS, Android, Linux, Windows, and the terminal that draw every built-in component, checked in CI on a real Mac, an iOS simulator, an Android emulator, a Linux desktop, a Windows desktop, and headless terminal sessions. Generated apps for all of those platforms. The Wasm runtime and code push, from a signed release to a verified, staged update on iOS and Android, optionally gated on independent source approvals. File routes, server functions, queries, translations, and the testing tools.

</Callout>

<Callout kind="important" title="Known gaps">

- **Generated Apple and Android apps** draw the common controls, including `Switch`, `Checkbox`, `Radio`, `Slider`, `LinearProgressIndicator`, and `ActivityIndicator`. Custom widgets remain unsupported. Native controls expose their checked and range states, but other accessibility roles and states remain limited. Android system Back is not yet connected to the file router.
- **iOS:** iOS has no native checkbox or radio button, so `Checkbox` and `Radio` are drawn with `UIButton`. VoiceOver gets fewer roles than macOS, and no expanded or collapsed state. [Live Activities](platforms/ios.md#live-activities-and-the-dynamic-island) start/update/end through ActivityKit on iOS only; the app must still supply `NSSupportsLiveActivities`, a Widget Extension for `LeptosKitLiveActivityAttributes`, and Push Notifications when using push tokens.
- **Android backend:** a `Custom` widget kind with no registered native implementation stops the app instead of rendering an empty container. TalkBack gets a smaller set of roles than macOS. Fold posture, dual-pane, and tabletop layouts work through the `posture` plugin; per-hinge angles on multi-hinge devices and live foldable-emulator checks are still thin — see [Foldable phones](guide/21-foldable-phones.md). The [`device-policy`](platforms/plugin-inventory.md#device-management) plugin covers Device Admin lock, password rules, camera disable, and confirmed wipe on Android only; apps own the receiver and least-privilege policy XML, and Google Play still constrains distribution. Device Owner / Android Management enrollment is not included.
- **Linux:** live regions update their text but are not announced, because announcements need GTK 4.14 and leptoskit supports GTK 4.6.
- **Windows:** `Switch` is drawn as a checkbox. Rounded corners are cut with a window region, so their edges are not smoothed, and borders stay rectangular inside them. Accessibility has no expanded or collapsed state. A `Vector` with no background takes clicks only where it has ink.
- **Desktop shell plugin:** invocation limits and cancellation clean up Unix process groups and Windows Job Objects. Unix commands that deliberately leave their group need additional host containment. Windows retains capacity when a member's termination cannot be confirmed; see the [plugin inventory](platforms/plugin-inventory.md).
- **Native menus:** builder APIs (`MenuBar`, `Submenu`, `Command`, `Check`) and macOS application-menu leases land in `leptoskit_plugin_menu`. Windows window-menu leases exist for hosts that own an HWND; Linux has no standalone menu bar (tray snapshots only). Standard roles, radio groups, icons, context menus, and GTK 4 window menus are still out. Generated Windows and Linux runners do not host the API yet. See [macOS application menus](platforms/macos.md#application-menus).
- **Home-screen widgets:** the [`home-widgets`](platforms/plugin-inventory.md) plugin publishes snapshots and reloads WidgetKit / Android App Widgets on iOS, Android, and macOS. It does not scaffold Widget extensions or App Widget providers, draw widget UI, or support interactive controls, multi-entry timelines, or Windows/Linux/browser backends yet.
- **Electron-shaped desktop lifecycle:** hosts can construct `power_monitor`, `deep_link`, `updater`, and second-launch handoff on `single_instance`. Idle-time queries are Windows-first; association writers, shipping update feeds, and desktop screenshot capture remain. See [Electron concepts](platforms/electron.md).
- **Rive:** [shared Rive elements](guide/20-rive.md) render on the web, macOS, iOS, Android, Linux, Windows, and experimental HarmonyOS. Native playback supports vector scenes and rejects images, fonts, text, layout, scripts, audio, view models, and unsupported draw features. View-model data binding is not exposed. HarmonyOS playback still needs validation on the target device.
- **Motion:** [`motion`](guide/32-motion.md) animates elements on the web only. macOS, iOS, Android, Windows, Linux, the terminal, and HarmonyOS show every element at its resting pose, and scroll-driven motion needs a browser with CSS scroll timelines; others show the resting pose too.
- **Runtime plugins on Windows, Linux, and the terminal:** the generated apps don't register runtime plugins yet. Plugins listed in `native.plugins` are left out with a warning, and calls to them fail with `Unavailable`. See [plugins a platform can't run](platforms/host-projects.md#plugins-a-platform-cant-run).
- **Picture-in-picture:** Android can enter activity PiP (with `android:supportsPictureInPicture` on the activity) and hold a system overlay after a settings grant for `SYSTEM_ALERT_WINDOW`. iOS reports PiP support but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`. See [runtime plugins](platforms/plugins.md).
- **App shortcuts, badges, and NFC:** [app shortcuts](platforms/plugin-inventory.md#launcher-presence-and-near-field-reads) publish static launcher items on iOS, Android, and a macOS dock-menu store; [badge](platforms/plugin-inventory.md#launcher-presence-and-near-field-reads) sets icon counts on Apple and Android, with a Linux D-Bus client for custom hosts; [NFC](platforms/plugin-inventory.md#launcher-presence-and-near-field-reads) reads one NDEF text or URI record on iOS and Android. Windows jump lists, Siri App Intents, tag writes, and the browser Badging API are still open.
- **HarmonyOS (experimental):** eight components (not including `Vector`), synchronous Rust plugins and ArkTS plugins (fold posture is the only ArkTS one so far) but not worker or async plugins, italic text and per-edge borders through ArkUI, and no code push, dark mode, right-to-left layout, `active:` pressed styles, or CI run on a device. See [HarmonyOS](platforms/harmonyos.md).
- **Terminal:** the terminal's font is the user's, so font sizes and weights draw as attributes and colors rather than different metrics, `Image` and `Vector` draw a placeholder texture, `Rive` playback is unsupported, and right-to-left text is not reordered. See [Terminal](platforms/tui.md).
- **Browser extensions:** [extensions](platforms/extension.md) build and package for Chromium, Firefox, and Safari, whose extension goes inside the macOS and iOS apps. `leptoskit dev` and `leptoskit test extension` run them in Chrome for Testing or Firefox (`--browser firefox`). [`leptoskit publish`](guide/24-publishing.md) uploads extensions to the Chrome Web Store, Edge Add-ons, and addons.mozilla.org, and apps to App Store Connect and Google Play. iOS apps carry their Safari extension; Mac App Store submission still needs a sandboxed host.
- **DevTools:** under `leptoskit dev`, the web page and the generated macOS, iOS, and Android apps report to [DevTools](guide/26-devtools.md#where-it-stands). The native apps outline the selected element and pick whatever you tap, but don't name each element's component yet. Windows, Linux, the terminal, and HarmonyOS don't report yet.
- **Scroll offsets:** [`scroll_offset`](guide/17-virtualized-lists.md#programmatic-scrolling) scrolls a list from code on the web and in the generated macOS, iOS, and Android apps. The native backend crates, and so the generated Windows and Linux apps, HarmonyOS, and the terminal don't apply it yet.
- **Plugins on Linux, Windows, and the terminal:** the generated runners can't host plugins yet. They build without the ones `leptoskit.toml` grants, and say so, so [sign-in](guide/23-sign-in.md) there answers that it isn't set up: its client needs the `http`, `secure-store`, `passkeys`, and `web-auth` plugins. Native apps also have no timer for the query cache yet, so retries there don't wait between attempts.
- **Sign-in:** email codes, authenticator apps (TOTP), recovery codes, step-up, required two-step verification, passkeys, opt-in passwords, the twenty built-in providers (Google, Apple, Microsoft, GitHub, Facebook, X, Discord, LinkedIn, GitLab, Slack, Twitch, Spotify, Reddit, Amazon, Dropbox, Bitbucket, Notion, Zoom, TikTok, and Yahoo), and any OpenID Connect provider work on the web and in the generated iOS, Android, and macOS apps. The apps use the system's own passkey sheet, Sign in with Apple's own sheet on Apple devices, and the system browser for other providers. In the apps, passkeys aren't offered in a text field's suggestions; Android signs in with providers only when its default browser has Auth Tab (Chrome 137 or later); and Apple devices need the app's associated domain, which the generated Xcode projects don't add yet. Windows and Linux have no native passkeys: a host that registers plugins itself gets provider sign-in in the default browser, and passkey calls answer `unsupported`. HarmonyOS has neither. Solana wallets link and sign in through the Wallet Standard on the web only; native apps list, rename, and unlink linked wallets, and answer `unsupported` for asking a wallet to sign until a native wallet adapter exists. Wallet proofs are message signatures, so program-derived addresses, multisigs, and wallets that can't sign messages can't be linked yet. `leptoskit generate auth` includes masked `PasswordInput` fields when password sign-in is enabled. Merging accounts can bring the other account's data along once every table that holds it declares an `on_account_merge` strategy, which `leptoskit db check` enforces.
- **Animations:** [transforms, transitions, keyframes, and entering and exiting animations](guide/32-animations.md) run on the platform's compositor on the web and in the generated iOS, Android, and macOS apps. The web doesn't run `exiting` or `layout_transition` yet. Windows, Linux, the terminal, HarmonyOS, and the native backend crates ignore them, so values jump.
- **Styled text, key handlers, and controlled focus** work on the web and in the terminal. The native widget backends still draw a `Text`'s styled runs in its own single style, deliver no `on_key` presses, and do not move focus for `focused(true)`. **Italic and per-edge borders** (`border-l-2`, `border-t`, …) paint on Linux (GTK), Android, and experimental HarmonyOS (ArkUI); AppKit, UIKit, and Win32 still draw italic upright and leave a per-edge border undrawn.

On every native platform, `Image` loads local files only, not URLs. Code push updates reach the iOS and Android apps; the generated desktop apps don't check for them yet. CSS shadows, gradients, filters, and custom fonts aren't supported anywhere yet: the [CSS coverage reference](reference/css-coverage.md) lists what is. Color and length `var(--name)` utilities work on the web and, through `Theme`, on native — including one nested fallback, `/opacity` on color vars, and `[theme]` tokens in `leptoskit.toml`. Deeper nesting and Tailwind `@theme` / `tailwind.config` files still don't. Rive's internal vector paints and transforms are separate from those CSS properties. [Platforms](platforms/readme.md) has the full picture.

</Callout>

Axum is the one supported server.

To work on leptoskit itself, start with [CONTRIBUTING.md](https://github.com/leptoskit/leptoskit/blob/main/CONTRIBUTING.md).
