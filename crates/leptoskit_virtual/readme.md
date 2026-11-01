<h1 align="center">leptoskit_virtual</h1>

<p align="center">
  <strong>leptoskit_virtual</strong> is headless list virtualization in the style of TanStack Virtual: fixed or measured sizes, overscan, aligned scroll targets, and a window over the shared renderer.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_virtual"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_virtual"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_virtual.svg"></a>
  <a href="https://docs.rs/leptoskit_virtual"><img alt="docs.rs" src="https://docs.rs/leptoskit_virtual/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Headless list virtualization for the leptoskit element tree, ported from [TanStack Virtual](https://tanstack.com/virtual): a `Virtualizer` computes exactly the items a scrolled viewport intersects — never the whole list — and `VirtualList` renders that window inside a `ScrollView` sized to the real total.

## Sizes

Sizes start from an estimate — one fixed number, or a function of the index — and resolve to real sizes through `measure`, which shifts every item after it and re-resolves only those. Each rendered item is sized to its size along the axis, so the layout matches the math. Overscan widens the window cheaply, `offset_for_index` computes aligned scroll targets, and an estimate that returns anything but a finite positive size is an error naming the index, never a silently clamped offset.

## Platforms

The core is axis-agnostic and pure math, so it runs anywhere the shared element tree does. The window follows the scroll events the platform reports: the web and every native backend report them; the generated macOS, iOS, and Android apps do not yet. Use the crate directly or enable `leptoskit`'s `virtual` feature and import `leptoskit::virtual_list::*`.

## How it fits

It decides which items a viewport intersects so a long list only builds the visible rows, on web and native alike. Infinite queries and tables use the same model.

## Use it

```rust
use leptoskit_native::components::Text;
use leptoskit_native::components::View;
use leptoskit_virtual::Axis;
use leptoskit_virtual::VirtualList;
use leptoskit_virtual::VirtualOptions;
use leptoskit_virtual::Virtualizer;

let mut virtualizer = Virtualizer::new(VirtualOptions::fixed(10_000, 56.0));
virtualizer.set_viewport(644.0);
virtualizer.scroll_to(0.0);

let list = VirtualList(&virtualizer, Axis::Vertical, |_offset| (), |item| {
	View().child(Text(format!("Contact {}", item.index)))
})
.expect("a fixed estimate is valid");
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_virtual)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Virtualized lists](https://ifiokjr.github.io/leptoskit/guide/17-virtualized-lists.html)
