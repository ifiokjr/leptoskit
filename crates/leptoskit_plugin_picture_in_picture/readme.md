<h1 align="center">leptoskit_plugin_picture_in_picture</h1>

<p align="center">
  <strong>leptoskit_plugin_picture_in_picture</strong> is the leptoskit plugin for cross-mobile picture-in-picture and Android system overlay windows.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#permissions"><strong>Permissions</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_picture_in_picture"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_picture_in_picture"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_picture_in_picture"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_picture_in_picture.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_picture_in_picture"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_picture_in_picture/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Keep a call, video, or similar surface visible while another app is open. Picture-in-picture shrinks the app into a system floating window. A system overlay is a separate window drawn over other apps; only Android supports that, and only after the user grants the special overlay permission in system settings.

Add the crate with its `guest` feature, select `picture-in-picture` in `native.plugins`, and run `leptoskit native prepare` to register the native handler.

Wire methods: `0` capabilities, `1` snapshot, `2` enter picture-in-picture (pending lease), `3` enter system overlay (pending lease), `4` overlay permission status, `5` request overlay permission (opens Android settings). Every request payload is empty.

Android enters activity picture-in-picture when the hosting activity declares `android:supportsPictureInPicture="true"`. Overlay entry creates a minimal `TYPE_APPLICATION_OVERLAY` window and removes it when the lease is dropped. Declaring `SYSTEM_ALERT_WINDOW` never grants access; `request_overlay` opens the system grant screen and returns the status observed at launch time.

iOS reports whether `AVPictureInPictureController` is supported. Entering PiP still needs an attached video player, which this release does not provide, so `enter_picture_in_picture` returns `Unavailable`. iOS has no draw-over-apps overlay API; overlay status is `Unsupported`. Desktop and the browser return `Unavailable`.

## How it fits

<!-- {=p_picture_in_pictureSummary} -->

Add `leptoskit_plugin_picture_in_picture` as a direct dependency and select the `picture-in-picture` plugin in `native.plugins`; supported generated hosts discover it from Cargo metadata. Generated Windows, Linux, and terminal hosts currently omit runtime plugins; use explicit registration in a custom host there. Guests enable the `guest` feature and reuse `guest::PictureInPicture` rather than resolving the plugin on every call. Android can enter activity picture-in-picture and, with `SYSTEM_ALERT_WINDOW`, hold a system overlay window. iOS reports whether system picture-in-picture is supported but does not yet enter it without an attached video player, and has no draw-over-apps overlay. Desktop and the browser return `Unavailable`.

<!-- {/p_picture_in_pictureSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_picture_in_picture::{
	OverlayPermission, guest::PictureInPicture,
};

let pip = PictureInPicture::new()?;
let capabilities = pip.capabilities().await?;

if capabilities.picture_in_picture {
	let lease = pip.enter_picture_in_picture()?;
	// Drop the lease when the floating session should end.
	drop(lease);
}

if capabilities.system_overlay {
	match pip.overlay_status().await? {
		OverlayPermission::Granted => {
			let lease = pip.enter_overlay()?;
			drop(lease);
		}
		OverlayPermission::Denied => {
			let _ = pip.request_overlay().await?;
		}
		OverlayPermission::Unsupported => {}
	}
}
```

## Permissions

| Platform | Requirement |
| -------- | ----------- |
| Android picture-in-picture | Activity attribute `android:supportsPictureInPicture="true"` (and usually `android:resizeableActivity="true"`). No runtime permission. |
| Android system overlay | Manifest permission `android.permission.SYSTEM_ALERT_WINDOW` (added from plugin metadata when selected). The user must grant it in system settings; use `request_overlay`. |
| iOS picture-in-picture | Background mode **Audio, AirPlay, and Picture in Picture** once a video player can enter PiP. This release only reports support. |
| iOS system overlay | Not available. |
| Desktop / browser | Not available. |

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_picture_in_picture)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
