<h1 align="center">leptoskit_plugin_deep_link</h1>

<p align="center">
  <strong>leptoskit_plugin_deep_link</strong> is the leptoskit plugin for inbound URL and file activations.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_deep_link"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_plugin_deep_link"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_deep_link"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_deep_link.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_deep_link"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_deep_link/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/leptoskit/leptoskit/ci.yml?label=CI"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Inbound deep links and file opens for desktop hosts. Construct `DeepLinks` with the URL schemes the app claims, optionally attach protocol and file association descriptors for packagers, and push activations when the OS delivers an open-URL or open-file event. The queue validates schemes against the allowlist, bounds path/URL size, and can be registered on the bridge for guests. Outbound browser/mail/phone links stay in `leptoskit_plugin_linking`. Declaring associations in Info.plist, the Windows registry, or a `.desktop` file remains a packaging step; this crate does not rewrite those files.

## How it fits

<!-- {=p_deep_linkSummary} -->

Native hosts construct `DeepLinks` with the schemes and file types they claim, push OS open-URL / open-file activations into it, and may expose it through the bridge. There is no automatic `native.plugins` registration. Outbound HTTP(S)/mailto/tel/sms opening remains `leptoskit_plugin_linking`. Packaging still has to declare the associations in Info.plist, the Windows registry, or a `.desktop` file; this crate holds the runtime queue and the association descriptors the host/packager use.

<!-- {/p_deep_linkSummary} -->

## Use it

```rust,ignore
use leptoskit_plugin_deep_link::DeepLinks;

let mut links = DeepLinks::new(["myapp".into()])?;
links.push_url("myapp://item/42")?;
let (_lost, events) = links.drain();
```

## Learn more

- [Electron → leptoskit map](https://leptoskit.dev/docs/platforms/electron.html)
- [Plugin inventory](https://leptoskit.dev/docs/platforms/plugin-inventory.html)
- [Repository](https://github.com/leptoskit/leptoskit)
