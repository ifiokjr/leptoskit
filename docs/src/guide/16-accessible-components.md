# Accessible components

The [accessibility guide](15-accessibility.md) covers the declarative model: every element carries WAI-ARIA roles, states, relationships, and live regions, and every backend maps them onto its platform. What that model leaves to each app is the widget behavior — the expanded state of a disclosure, the selection of a tab list, the label, help, and error wiring of a field. `leptoskit_aria` is that layer, in the spirit of react-aria and [leptos_aria](https://github.com/ifiokjr/leptos_aria): the component owns the semantics, the app owns the styling through the same Tailwind classes every leptoskit component takes.

```toml
[dependencies]
leptoskit_aria = "0.1"
```

Or enable `leptoskit`'s `aria` feature and import `leptoskit::aria::*`.

## The contract

Components are plain functions, or small values like `Field`, `Tabs`, and `Dialog`, built from current state plus callbacks. A screen owns its state — signals, atomics, or otherwise — and rebuilds the tree when it changes; each component renders the state it is given. The parts of one widget are separate elements, so layout stays the app's choice, and compound parts share a caller-chosen id prefix: the button and panel of a disclosure both derive their ids from the same `"faq"` prefix.

The renderer's validation enforces the wiring these components declare. A field whose input is labelled by an id no element carries, or a dialog without its title, is a tree that fails to commit. Wrong accessibility is a visible error, not a screen-reader-only surprise.

```rust,ignore
use leptoskit::component;
use leptoskit::view;
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

#[component]
fn Faq(open: bool, toggle: impl Fn(bool) + Send + Sync + 'static) -> leptoskit_native::Element {
	view! {
		<View>
			{DisclosureButton("faq", "Frequently asked questions", open, toggle)}
			{DisclosurePanel("faq", open).child(view! {
				<Text>"Only visible once expanded"</Text>
			})}
		</View>
	}
}
```

## The components

| Component                              | Contract                                                                    |
| -------------------------------------- | --------------------------------------------------------------------------- |
| `DisclosureButton` / `DisclosurePanel` | Expand and collapse a region, wired by `aria-expanded` and `aria-controls`. |
| `ToggleButton`                         | A pressed-state button, including the mixed state.                          |
| `Tabs`                                 | Tabs with selection, set position, and panel relationships.                 |
| `Field`                                | Label, help text, and error wiring for any input element.                   |
| `Dialog`                               | A dialog container named by its title and described by its description.     |

### Disclosure

The button names the action, carries `aria-expanded`, and references its panel with `aria-controls`; the panel is a region labelled by the button that stays mounted while collapsed, so the relationship always resolves and collapsed content is unreachable rather than absent. Pressing the button calls `on_toggle` with the proposed state; keeping it is the caller's, like every controlled control in leptoskit.

### ToggleButton

`aria-pressed` on a plain button, with the WAI-ARIA mixed-state convention: pressing an unpressed or mixed button proposes `true`, pressing a pressed one proposes `false`. For a two-state control that also works with switch announcements, `Switch` remains the native primitive.

### Tabs

One `Tabs` describes a tab set — its id prefix, how many tabs it has, and which one is selected — and builds every part: `tabs.list()` for the `tablist` container, `tabs.tab(index, label, on_select)` for each tab, and `tabs.panel(index)` for the panel it selects. Because every part comes from the same value, exactly one tab is selected, every tab reports the same set size, and each tab's `aria-controls` names its own panel; an index outside the set panics. Panels stay mounted with the inactive ones collapsed, so switching keeps panel state.

Every tab stays in the tab order. The WAI-ARIA roving tabindex keeps only the selected tab focusable and moves between tabs with the arrow keys, and the renderer has no key events yet, so roving would leave unselected tabs out of keyboard reach. Enter and Space activate the focused tab; arrow-key motion and roving follow the focus work tracked in the accessibility guide.

### Field

One `Field` names an input and carries its help text and error. Place the label, help, and error elements around the input; `field.wire(input)` gives the input the field's id and connects them with `aria-labelledby` and `aria-describedby` — appended to any description the input already carries — and marks the value `aria-invalid` while an error is set. Build a fresh `Field` each render from current state so an error that appears rewires the input; announce the new error with an `Alert` when it should interrupt.

### Dialog

One `Dialog` carries a dialog's id, title, and optional description, and builds the container with `dialog.container(open)`, the title with `dialog.title_element(level)`, and the description with `dialog.description_element()`. The container is labelled by the title and, when a description is set, described by it, so the references cannot drift; validation rejects a container whose title element is missing. The dialog stays mounted while closed, so its references always resolve.

The container is not marked `aria-modal`. A modal dialog promises that focus stays inside it, and focus containment — trapping and restoring focus — is still pending in the renderer, so the promise would hide the page from screen readers while keyboard focus still leaves. An app that contains focus itself can chain `.aria_modal(true)`. Until containment lands, pair a dialog with an assertive live region for its announcement.

## Testing

The components are ordinary element trees, so the testing guide applies directly:

- `Finder::aria_role(Role::Tab, "Notifications")` finds tabs by role and name, and `tester.tap` drives selection through the real event dispatch.
- `WidgetTester::semantics()` prints what assistive technology sees — each tab's selection and position in the set show up in the tree.
- Wiring failures are `Element::validate()` errors: a test that builds a dialog without its title asserts the invalid tree up front.

## What comes next

More widget families — menus, list boxes with selection, accordion (a disclosure group with exclusive opening), tooltips, and announcements outside a live region — follow the same pattern on top of the renderer's pending focus work: scopes, traps, and restoration, plus custom accessibility actions. Each one is behavior and semantics over the same shared tree, not a new platform surface.
