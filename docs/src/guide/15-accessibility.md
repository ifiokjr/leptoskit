# Accessibility

Accessibility in leptoskit is ARIA. The props are the WAI-ARIA names, the web renderer emits them as real attributes and semantic elements, and every native backend maps the same properties onto its platform: VoiceOver on iOS and macOS, TalkBack on Android, UI Automation on Windows, and AT-SPI through GTK. React Native converged on the same `aria-*` vocabulary, and GTK's accessible API mirrors it almost one to one, so one shared model covers every platform without a web-only escape hatch.

## The props

Every element carries the same builder methods:

| Builder                                   | Web attribute                    | What it does                                                                       |
| ----------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------------- |
| `aria_label`                              | `aria-label`                     | The accessible name, announced first. Overrides visible text.                      |
| `aria_labelled_by(["id"])`                | `aria-labelledby`                | Names the element after other elements, by `id`.                                   |
| `aria_description`                        | `aria-description`               | A description beyond the name.                                                     |
| `aria_described_by(["id"])`               | `aria-describedby`               | Describes the element with other elements; the usual home of help text and errors. |
| `role(Role::Heading)`                     | `role`                           | Overrides the widget kind's implicit role.                                         |
| `aria_level(n)`                           | `aria-level`                     | Heading, list, or tree depth, from one.                                            |
| `aria_hidden(true)`                       | `aria-hidden`                    | Hides the subtree from assistive technology while it stays visible.                |
| `aria_checked(Tristate::Mixed)`           | `aria-checked`                   | Check state of a custom checkbox, option, or switch.                               |
| `aria_pressed(true)`                      | `aria-pressed`                   | Toggle-button state.                                                               |
| `aria_selected` / `aria_expanded`         | same                             | Selection and expansion state.                                                     |
| `aria_current(Current::Page)`             | `aria-current`                   | Marks the current page, step, or item.                                             |
| `aria_value_now/min/max`                  | `aria-valuenow` and friends      | Range values for custom range widgets.                                             |
| `aria_value_text("50 percent")`           | `aria-valuetext`                 | A human-readable range value.                                                      |
| `aria_live(Live::Polite)`                 | `aria-live`                      | Makes the element a live region whose text changes are announced.                  |
| `aria_atomic` / `aria_busy`               | same                             | Whether the whole region is announced; whether it is mid-change.                   |
| `aria_modal`                              | `aria-modal`                     | A dialog that blocks the rest of the page.                                         |
| `aria_required` / `aria_invalid`          | same                             | Field requirements and validity.                                                   |
| `aria_read_only` / `aria_multiline`       | same                             | Field editing traits.                                                              |
| `aria_has_popup(HasPopup::Menu)`          | `aria-haspopup`                  | What a control opens.                                                              |
| `aria_controls(["id"])`                   | `aria-controls`                  | Links a control to what it controls.                                               |
| `aria_pos_in_set(n)` / `aria_set_size(n)` | `aria-posinset` / `aria-setsize` | Position in a set; radios fill these automatically.                                |
| `aria_orientation`                        | `aria-orientation`               | A widget's axis.                                                                   |
| `aria_role_description("slide")`          | `aria-roledescription`           | A human-readable role label.                                                       |
| `tab_index(0)`                            | `tabindex`                       | Joins or leaves sequential keyboard focus.                                         |
| `id("name")`                              | `id`                             | A stable identifier, for relationships and the DOM.                                |
| `aria_attr("aria-rowindex", "3")`         | verbatim                         | Any other `aria-*` attribute, emitted on the web, ignored natively.                |

`accessibility_label` remains as an alias of `aria_label`.

## Roles become semantic elements

The first rule of ARIA is to prefer native HTML, and the web renderer does: structural roles upgrade the element instead of adding a redundant `role` attribute.

```rust,ignore
Text("Profile").role(Role::Heading).aria_level(2)   // <h2>Profile</h2>
Heading(2, "Profile")                                // the same, as a component
View().role(Role::List)                              // <ul>
View().role(Role::Navigation)                        // <nav>
View().role(Role::Banner)                            // <header>
View().role(Role::Main)                              // <main>
View().role(Role::Separator)                         // <hr> without children
Divider()                                            // a separator with orientation
```

Every `NativeKind` implies a role — a `Button` is a `button`, a `TextInput` a `textbox`, a `Switch` a `switch` — and an explicit `role` overrides it. A container that declares a control role joins the tab order and activates from Enter and Space, because no native element does it for it:

```rust,ignore
View()
    .role(Role::Button)
    .aria_label("Archive order")
    .on_press(archive)
```

On native platforms, the same declarations map onto VoiceOver traits, TalkBack class names, NSAccessibility roles, UIA control types, and GTK accessible roles. Unmapped roles read as a group with the ARIA name as the role description.

## Naming and validation

Some contracts are checked before any tree commits, like radio groups and pressable labels:

- Text inputs need an accessible name: a label, a labelling reference, or at least a placeholder. A placeholder alone is the documented fallback name on every platform, but it disappears when typing — prefer a real label.
- Ids are unique within a tree, and relationship references point at ids that exist.
- A presentational role (`none`/`presentation`) cannot carry a name.
- `aria_level` starts at one, and passthrough `aria_attr` names must be lowercase `aria-*` attributes without a typed builder.

## Relationships and merging

`aria_labelled_by` and `aria_described_by` reference element ids. On the web the renderer keeps the id references and browsers compute the name live. Native accessibility APIs have no id references, so the renderer resolves them: referenced text is folded into the label before backends see it, and radio options learn their position in the group automatically.

A plain container with a `role` or an `aria_label` becomes one accessibility element, merging its content the way React Native's `accessible` does — VoiceOver and TalkBack read the card below as a single item:

```rust,ignore
View().aria_label("Order card").child(Text("Order #42"))
```

## Live regions

Live regions announce their changing text: politely once the reader is idle, assertively by interrupting.

```rust,ignore
Status("3 items in your cart")   // role=status: polite, atomic
Alert("Connection lost")         // role=alert: assertive
```

The web emits the attributes and the browser announces changes. Android uses its native live region. iOS, macOS, and Windows announce when the resolved region text changes, at high priority for assertive regions. GTK exposes the text through the label; announcements need GTK 4.14, above this crate's current 4.6 minimum.

## Images

`Image` is decorative until named: the web renders `alt=""` and native backends leave it out of the accessibility tree. Name informative images with `aria_label`, and hide purely decorative subtrees with `aria_hidden(true)`.

## Testing

- `Finder::aria_role(Role::Button, "Continue")` finds elements by effective role and accessible name, like Testing Library's `getByRole` — the role a widget kind implies counts.
- `WidgetTester::semantics()` prints the semantics tree a screen reader would expose — roles, names, states, values, live politeness — for assertions and snapshots, like Flutter's semantics tree dump.
- Server-rendered HTML tests assert the real attributes; the browser suite exercises keyboard activation; the AppKit suite asserts NSAccessibility state; and the native testing harness drives real devices through their accessibility drivers.

## Limits and what comes next

Widget behaviors — disclosure, toggle buttons, tabs, field wiring, dialogs — now ship as headless components in `leptoskit_aria`; see the [accessible components guide](16-accessible-components.md). Focus scopes, traps, and restoration — the prerequisite for containing dialog focus — are still pending, as are custom accessibility actions (React Native's `accessibilityActions`, Flutter's `customSemanticsActions`). Runtime services such as announcements outside a live region, screen-reader detection, reduce-motion, and font-scale preferences belong in a follow-up `leptoskit_plugin_accessibility` runtime plugin, modelled on the speech plugin; the declarative model needs no extra library.
