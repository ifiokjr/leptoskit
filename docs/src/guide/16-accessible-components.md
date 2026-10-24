# Accessible components

The [accessibility guide](15-accessibility.md) covers the declarative model: every element carries WAI-ARIA roles, states, relationships, and live regions, and every backend maps them onto its platform. What that model leaves to each app is the widget behavior — the expanded state of a disclosure, the roving focus of a tab list, the label, help, and error wiring of a field. `leptoskit_aria` is that layer, in the spirit of react-aria and [leptos_aria](https://github.com/ifiokjr/leptos_aria): the component owns the semantics, the app owns the styling through the same Tailwind classes every leptoskit component takes.

```toml
[dependencies]
leptoskit_aria = "0.1"
```

Or enable `leptoskit`'s `aria` feature and import `leptoskit::aria::*`.

## The contract

Components are plain functions of current state plus callbacks. A screen owns its state — signals, atomics, or otherwise — and rebuilds the tree when it changes; each component renders the state it is given. The parts of one widget are separate elements, so layout stays the app's choice, and compound parts share a caller-chosen id prefix: the button and panel of a disclosure both derive their ids from the same `"faq"` prefix.

The renderer's validation enforces the wiring these components declare. A field whose input is labelled by an id no element carries, or a dialog without its title, is a tree that fails to commit. Wrong accessibility is a visible error, not a screen-reader-only surprise.

```rust,ignore
use leptoskit_aria::DisclosureButton;
use leptoskit_aria::DisclosurePanel;
use leptoskit_native::components::Text;
use leptoskit_native::components::View;

fn faq(open: bool, toggle: impl Fn(bool) + Send + Sync + 'static) -> leptoskit_native::Element {
	View()
		.child(DisclosureButton("faq", "Frequently asked questions", open, toggle))
		.child(DisclosurePanel("faq", open).child(Text("Only visible once expanded")))
}
```

## The components

| Component                                      | Contract                                                                    |
| ---------------------------------------------- | --------------------------------------------------------------------------- |
| `DisclosureButton` / `DisclosurePanel`         | Expand and collapse a region, wired by `aria-expanded` and `aria-controls`. |
| `ToggleButton`                                 | A pressed-state button, including the mixed state.                          |
| `TabList` / `Tab` / `TabPanel`                 | Tabs with roving focus, selection, and panel relationships.                 |
| `Field`                                        | Label, help text, and error wiring for any input element.                   |
| `Dialog` / `DialogTitle` / `DialogDescription` | A modal dialog container named by its title.                                |

### Disclosure

The button names the action, carries `aria-expanded`, and references its panel with `aria-controls`; the panel is a region labelled by the button that stays mounted while collapsed, so the relationship always resolves and collapsed content is unreachable rather than absent. Pressing the button calls `on_toggle` with the proposed state; keeping it is the caller's, like every controlled control in leptoskit.

### ToggleButton

`aria-pressed` on a plain button, with the WAI-ARIA mixed-state convention: pressing an unpressed or mixed button proposes `true`, pressing a pressed one proposes `false`. For a two-state control that also works with switch announcements, `Switch` remains the native primitive.

### Tabs

`TabList` contributes the role; each `Tab` carries selection, its `aria-controls` panel reference, its position in the set, and the roving tabindex — the selected tab at `0`, the rest at `-1`. Panels stay mounted with the inactive ones collapsed, so switching keeps panel state. The web renderer activates tabs from Enter and Space; arrow-key motion between tabs follows the focus work tracked in the accessibility guide.

### Field

One `Field` names an input and carries its help text and error. Place the label, help, and error elements around the input; `field.wire(input)` connects them with `aria-labelledby` and `aria-describedby` and marks the value `aria-invalid` while an error is set. Build a fresh `Field` each render from current state so an error that appears rewires the input; announce the new error with an `Alert` when it should interrupt.

### Dialog

A dialog must be named: the container references its `DialogTitle`, and validation rejects a dialog without one. `aria-modal` is set, and the dialog stays mounted while closed, so the title id always resolves. Focus containment — trapping and restoring focus — is still pending in the renderer, so pair a dialog with an assertive live region for its announcement until that lands.

## Testing

The components are ordinary element trees, so the testing guide applies directly:

- `Finder::aria_role(Role::Tab, "Notifications")` finds tabs by role and name, and `tester.tap` drives selection through the real event dispatch.
- `WidgetTester::semantics()` prints what assistive technology sees — the roving tab order shows up as each tab's `tabindex` state.
- Wiring failures are `Element::validate()` errors: a test that builds a dialog without its title asserts the invalid tree up front.

## What comes next

More widget families — menus, list boxes with selection, accordion (a disclosure group with exclusive opening), tooltips, and announcements outside a live region — follow the same pattern on top of the renderer's pending focus work: scopes, traps, and restoration, plus custom accessibility actions. Each one is behavior and semantics over the same shared tree, not a new platform surface.
