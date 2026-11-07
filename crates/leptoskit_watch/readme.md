<h1 align="center">leptoskit_watch</h1>

<p align="center">
  <strong>leptoskit_watch</strong> is the shared watch UI surface for Android Wear OS and Apple watchOS: constrained pages, tiles, glances, and complications that reuse leptoskit styling without treating a phone screen as a watch.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="https://docs.rs/leptoskit_watch"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_watch"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_watch"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_watch.svg"></a>
  <a href="https://docs.rs/leptoskit_watch"><img alt="docs.rs" src="https://docs.rs/leptoskit_watch/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Phone apps and watch apps are different products. A Wear OS tile or a watchOS complication is a tiny ambient surface, not a phone route scaled down. This crate gives watch apps their own containers — `WatchPage`, `WatchTile`, `WatchGlance`, `WatchComplication` — that still take leptoskit `Element` trees for content, so Tailwind layout and the shared primitives (`View`, `Text`, `Button`, …) keep working inside a hard budget.

Both platform backends (`leptoskit_wearos`, `leptoskit_watchos`) implement [`WatchHost`]. On hosts that are not yet wired to a device toolkit, presentations are recorded in memory so tests and cross-compiling guests still build.

## Use it

```rust
use leptoskit_native::components::Button;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_watch::WatchApp;
use leptoskit_watch::WatchKind;
use leptoskit_watch::WatchNavigation;
use leptoskit_watch::WatchPage;

let page = WatchPage::new(
	"home",
	View()
		.class("items-center justify-center gap-2 p-2")
		.child(Text("Steps").class("text-sm"))
		.child(Text("8,432").class("text-2xl font-bold"))
		.child(Button("Open")),
);
let app = WatchApp::new("steps", WatchKind::WearOs)
	.navigation(WatchNavigation::Stack)
	.page(page);
assert!(app.validate().is_ok());
```

Wear OS and watchOS support is experimental. See the platform pages in the docs for entitlements, host stubs, and what is not packaged yet.
