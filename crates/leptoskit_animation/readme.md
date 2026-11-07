<h1 align="center">leptoskit_animation</h1>

<p align="center">
  <a href="https://leptoskit.dev/"><img src="https://raw.githubusercontent.com/leptoskit/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_animation</strong> describes animations once, as data, so each platform runs them on its own compositor instead of in your app's code.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_animation"><strong>Docs</strong></a> ·
  <a href="https://github.com/leptoskit/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_animation"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_animation"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_animation.svg"></a>
  <a href="https://docs.rs/leptoskit_animation"><img alt="docs.rs" src="https://docs.rs/leptoskit_animation/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/leptoskit/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/leptoskit/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

A leptoskit app's interface runs as a Wasm guest in an interpreter. If the guest computed every frame of an animation, a busy guest would stutter it. Instead the guest describes the animation once, as part of a widget's properties, and the platform runs it: Core Animation on macOS and iOS, the Android animation framework, and CSS transitions and animations on the web. React Native Reanimated runs its animations on the UI thread for the same reason.

- `Curve`: a timed easing (`Curve::timing`) or a damped `Spring` (`Curve::spring`), with presets.
- `Transition`: animate opacity, transform, and color changes instead of jumping.
- `Transform`: move, scale, and rotate a widget's drawing about its center, by points or by a percentage of its own size.
- `Keyframes`: looping or one-off sequences, including Tailwind's `spin`, `ping`, `pulse`, and `bounce`.
- `Presence`: how a widget enters when added and exits when removed (`fade`, `slide`, `zoom`, and combinations).
- `Animation`: all of these for one widget.

Every type has a `sanitized` form, so values from an untrusted guest stay finite and bounded.

## How it fits

`leptoskit_native` carries these values in a widget's props and builds them from Tailwind classes (`transition`, `duration-*`, `animate-spin`, `starting:`), and the web adapter turns them into CSS. The Swift and Kotlin renderers in `leptoskit_bridge_ffi` read them from each render transaction and run them natively. The math every platform shares, such as spring progress and keyframe sampling, lives here.

## Use it

```rust
use leptoskit_animation::Edge;
use leptoskit_animation::Presence;
use leptoskit_animation::Spring;
use leptoskit_animation::Transition;

// Fade in while rising, and slide out to the left on a spring.
let entering = Presence::fade().slide_by(0.0, 16.0);
let exiting = Presence::slide(Edge::Left).spring(Spring::SNAPPY);
let transition = Transition::spring(Spring::GENTLE);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_animation)
- [Animations guide](https://leptoskit.dev/docs/guide/32-animations.html)
- [leptoskit documentation](https://leptoskit.dev/docs/)
- [Repository](https://github.com/leptoskit/leptoskit)
