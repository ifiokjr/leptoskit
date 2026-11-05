<h1 align="center">leptoskit_aria</h1>

<p align="center">
  <strong>leptoskit_aria</strong> provides headless accessible components for the shared element tree: disclosure, toggle buttons, tabs, labeled fields, and dialogs with roles and state wired for web and native.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_aria"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<!-- {=crateBadges|replace:"CRATE_NAME":"leptoskit_aria"} -->

<p align="center">
  <a href="https://crates.io/crates/leptoskit_aria"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_aria.svg"></a>
  <a href="https://docs.rs/leptoskit_aria"><img alt="docs.rs" src="https://docs.rs/leptoskit_aria/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<!-- {/crateBadges} -->

<br>

## Overview

Headless accessible components for the leptoskit element tree, in the spirit of [react-aria](https://react-spectrum.adobe.com/react-aria/) and [leptos_aria](https://github.com/ifiokjr/leptos_aria): the component owns the semantics — roles, states, relationships, and live regions — and the app owns the styling, through the same Tailwind classes every leptoskit component takes.

Every component emits roles, states, and relationships in the shared `Element` tree. The web renderer and native backend crates consume this model with platform-specific limits. Generated macOS, iOS, and Android apps currently forward accessible names but do not forward most roles and states; emitting ARIA metadata alone does not provide complete accessibility on those hosts. Compound parts share a stable id prefix, and the renderer's own validation enforces the wiring: a tree that references a missing label or panel id fails to commit.

Use the crate directly or enable `leptoskit`'s `aria` feature and import `leptoskit::aria::*`.

## Components

| Component                              | Contract                                                                    |
| -------------------------------------- | --------------------------------------------------------------------------- |
| `DisclosureButton` / `DisclosurePanel` | Expand and collapse a region, wired by `aria-expanded` and `aria-controls`. |
| `ToggleButton`                         | A pressed-state button, including the mixed state.                          |
| `Tabs`                                 | Tabs with selection, set position, and panel relationships.                 |
| `Field`                                | Label, help text, and error wiring for any input element.                   |
| `Dialog`                               | A dialog container named by its title and described by its description.     |

Components are plain functions, or small values like `Field`, `Tabs`, and `Dialog`, built from current state plus callbacks, matching the leptoskit rebuild model: screens own state — signals, atomics, or otherwise — and re-render the tree when it changes. The parts are separate elements so layout stays the app's choice.

## How it fits

<!-- {=ariaSummary} -->

It owns behavior, not pixels: app code styles the pieces with Tailwind and gets shared focus, keyboard, and ARIA metadata. Platform renderers determine which semantics reach assistive technology. `leptoskit_form` builds its fields on top of it.

<!-- {/ariaSummary} -->

## Use it

```rust
use leptoskit::view;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_native::components::View;

let screen = view! {
	<View>
		{DisclosureButton("faq", "Frequently asked questions", false, |_| ())}
		{DisclosurePanel("faq", false)}
	</View>
};
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_aria)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Accessible components](https://ifiokjr.github.io/leptoskit/guide/16-accessible-components.html)
- [Accessibility](https://ifiokjr.github.io/leptoskit/guide/15-accessibility.html)
