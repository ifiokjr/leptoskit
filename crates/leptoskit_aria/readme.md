# leptoskit_aria

Headless accessible components for the leptoskit element tree, in the spirit of [react-aria](https://react-spectrum.adobe.com/react-aria/) and [leptos_aria](https://github.com/ifiokjr/leptos_aria): the component owns the semantics — roles, states, relationships, and live regions — and the app owns the styling, through the same Tailwind classes every leptoskit component takes.

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

Every component emits the shared `Element` tree, so one widget works on the web renderer and on every native backend: VoiceOver on iOS and macOS, TalkBack on Android, UI Automation on Windows, and AT-SPI through GTK. Compound parts share a stable id prefix, and the renderer's own validation enforces the wiring: a tree that references a missing label or panel id fails to commit.

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
