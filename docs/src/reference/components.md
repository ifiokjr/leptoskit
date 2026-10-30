# Component catalog

Leptoskit's primary components are native primitive builders exported by `leptoskit::components` and `leptoskit::prelude`. Each returns an `Element` and understands Tailwind. `View`, `Text`, `Image`, `ScrollView`, editable inputs, buttons, and native controls provide the building blocks for shared screens.

The primary API has 15 primitive builders. Another 31 layout and presentation builders are optional helpers under `leptoskit::components::helpers`. Import them explicitly when useful. They compose the same primitives and introduce no native widget kinds. New primitive behavior follows React Native and Lynx; the contracts below do not reproduce every upstream property.

`View()` is a column container, `Text(content)` displays inherited system-font text, `Image(source)` displays a host-resolvable image, `Vector(view_box)` draws filled and stroked paths, and `ScrollView()` contains scrolling children. The controls section lists the other primitives.

## Optional layout and presentation helpers

Use `View` plus Tailwind for rows, grids, and layered content. Helpers remain available through an explicit import such as `use leptoskit::components::helpers::Stack;`. The table includes the basic primitives for comparison; all other builders in this section belong to `helpers`.

| Builder                                | Contract                                                                                                                                                             |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `View()`                               | A column container.                                                                                                                                                  |
| `Text(content)`                        | A text label with inherited system-font styling.                                                                                                                     |
| `Image(source)`                        | An image from a host-resolvable source. AppKit accepts local paths and file URLs. Web accepts browser URLs.                                                          |
| `Rive(playback)`                       | An owned [Rive vector animation](../guide/20-rive.md), behind the `rive` feature. Requires an explicit size and accessible label or decorative hiding.               |
| `Row()`, `Column()`, `Flex(direction)` | Flex containers. Children do not shrink by default. `FlexDirection` includes reversed directions.                                                                    |
| `Wrap()`                               | A horizontal flex container with wrapping and start-aligned runs. Tailwind controls direction, gaps, and alignment.                                                  |
| `Grid(columns)`                        | Equal-width `minmax(0, 1fr)` columns and automatic rows. The count is `NonZeroU8`.                                                                                   |
| `Padding(insets)`                      | A wrapper with `Rect<Dimension>` padding. Percentages resolve against the containing width. `Auto` means zero.                                                       |
| `SizedBox(width, height)`              | Explicit wrapper dimensions. `Dimension::Auto` leaves an axis automatic.                                                                                             |
| `ConstrainedBox(constraints)`          | Validated CSS minimum and maximum dimensions on the wrapper.                                                                                                         |
| `AspectRatio(ratio)`                   | A preferred width-to-height ratio. At least one definite axis is needed to derive the other. Explicit dimensions and minimum or maximum constraints take precedence. |
| `Align(alignment)`, `Center()`         | Nine-position alignment within the wrapper's allocated box. Multiple children form a column. Start and end follow the wrapper's direction.                           |
| `Expanded(child)`                      | Modifies the supplied child to use `flex: 1 1 0` with zero minimum dimensions. It adds no wrapper.                                                                   |
| `Spacer()`                             | Empty expanded flex space.                                                                                                                                           |
| `Stack(children)`                      | Overlaps the supplied collection in one grid cell. Non-positioned children determine intrinsic size. Later children paint above earlier children.                    |
| `Positioned(insets, child)`            | Modifies the supplied child to use absolute positioning. In a stack, the grid cell defines its containing block. In a flex container, the parent's padding box does. |
| `ColoredBox(color)`                    | A solid background.                                                                                                                                                  |
| `ClipRect()`, `ClipRRect(radius)`      | Rectangular clipping or clipping with a uniform corner radius.                                                                                                       |
| `Opacity(value)`                       | Composites the subtree at a uniform opacity. Zero opacity preserves layout and input.                                                                                |
| `DefaultTextStyle(style)`              | Inherited font size, weight, and text alignment. `.color(...)` supplies an inherited foreground.                                                                     |
| `Visibility(visible)`                  | Collapses the subtree when false. Nodes remain reconciled, but hidden descendants take no space and receive no input.                                                |
| `Divider()`, `VerticalDivider()`       | One-pixel separators that stretch across the appropriate cross axis.                                                                                                 |
| `Card()`                               | A padded white container with rounded corners. It has no elevation or implicit action.                                                                               |
| `Badge(label)`, `Chip(label)`          | Decorative labels. Chips have no selection, deletion, or press behavior.                                                                                             |
| `ListTile(leading, content, trailing)` | A noninteractive row with optional leading and trailing slots and an expanded content slot. Slots can contain real controls.                                         |
| `AppBar(title)`                        | A header with an expanded title. Additional children are action slots. It does not control window chrome.                                                            |
| `Scaffold(header, body, footer)`       | A column with optional header and footer and an expanded body. It needs a definite height or a parent allocation.                                                    |
| `ScrollView()`                         | A scroll container.                                                                                                                                                  |
| `ListView()`, `GridView(columns)`      | Eager scrolling collections that fill the available height in a bounded column. Every item is constructed and laid out.                                              |

`BoxConstraints::new(min_width, max_width, min_height, max_height)` accepts finite nonnegative minima and optional finite maxima. A maximum cannot be smaller than its minimum. `BoxConstraints::tight(width, height)` fixes both dimensions. These constraints apply to the CSS box, rather than Flutter's parent-to-child constraint protocol.

`AspectRatio`, `ClipRRect`, and `Opacity` return `Result<Element, LayoutError>`. Ratios range from `1e-6` through `1e6`. Radii must be finite and nonnegative. Opacity must be finite and between zero and one.

`Align` has no Flutter `widthFactor` or `heightFactor`. Give the wrapper dimensions or a flex allocation when it needs extra space. `Expanded` must be a direct child of a flex container. It implements a tight flex allocation, with no `Flexible` loose-fit equivalent.

`Stack` assigns overlap placement to the collection passed to its constructor. Appending `.child(...)` afterward uses ordinary grid placement. Positioned children do not contribute to intrinsic size. A stack containing only positioned children needs explicit dimensions or a parent allocation.

## Controls

| Builder                             | Shared value and events                                      | AppKit widget                             | Web element                       |
| ----------------------------------- | ------------------------------------------------------------ | ----------------------------------------- | --------------------------------- |
| `Button(label)`                     | `.on_press(Fn())`                                            | `NSButton`                                | `button`                          |
| `Pressable(label)`                  | Accessible label and decorative children. `.on_press(Fn())`. | Flipped `NSButton` with child hit routing | `button`                          |
| `TextInput()`                       | `.value(...)`, `.on_change(Fn(&str))`                        | `NSTextField`                             | Text `input`                      |
| `PasswordInput()`                   | The same single-line text contract, with masked display      | `NSSecureTextField`                       | Password `input`                  |
| `Switch(checked)`                   | `.on_toggle(Fn(bool))`                                       | `NSSwitch`                                | Checkbox `input` with switch role |
| `Checkbox(checked)`                 | `.on_toggle(Fn(bool))`                                       | Switch-type `NSButton`                    | Checkbox `input`                  |
| `Radio(group, checked)`             | `.on_toggle(Fn(bool))` proposes selection                    | Radio-type `NSButton`                     | Radio `input`                     |
| `Slider(state)`                     | `.on_value_change(Fn(f64))`                                  | `NSSlider`                                | Range `input`                     |
| `LinearProgressIndicator(progress)` | Determinate fraction or indeterminate animation              | Bar `NSProgressIndicator`                 | `progress`                        |
| `ActivityIndicator(running)`        | Running or stopped animation                                 | Spinning `NSProgressIndicator`            | Styled `span` with progress role  |

Applications own control values. Handlers receive proposed values. Rebuilding the tree commits an accepted value. Both adapters restore the last committed value when the application rejects a proposal, including a proposal that triggers no rebuild.

`RadioGroup::new(name)` validates a nonempty name of at most 128 bytes with no control characters. A group is scoped to one rendered tree, and at most one member can be selected. Applications update all related radio values together. Arrow keys move among enabled group members and propose selection.

`SliderState::new(min, max, value, step)` validates finite increasing bounds and an in-range value. `None` selects continuous values. A step must be positive and no larger than the range. At the endpoint with the largest magnitude, a step must span at least 16 adjacent representable `f64` values. This precision margin makes stepping and reconstruction stable throughout the range. A stepped value must lie on `min + n * step`, with one representable value of tolerance for rounding. The upper bound need not lie on that grid. Native, web, wire decoding, and the widget tester use the same quantization.

`Progress::Determinate(ProgressValue::new(fraction)?)` validates a finite fraction between zero and one. `Progress::Indeterminate` animates without a completion value. Hidden or stopped activity indicators do not animate. Browser animation respects reduced-motion preferences.

Checkboxes support boolean state. Mixed state, vertical sliders, multiple slider thumbs, multiline text, text selection APIs, and input-purpose configuration are not implemented. Password display masking does not encrypt application state or wire data.

`Pressable` provides activation, keyboard focus, an accessible name, and decorative child content. Pointer activation stays within its bounds. While it is held down, it shows its `active:` utilities, or dims without them, as `Button` does; [Style the pressed state](../guide/07-shared-components.md#style-the-pressed-state) explains when each platform starts and ends a press. Handlers don't receive press-in or press-out, and there are no long-press, hit-slop, or gesture cancellation events. Its accessible label must contain a non-whitespace character. Its content cannot contain interactive widgets, event handlers, custom platform widgets, or scrolling descendants. `Button` also rejects nested controls and scrolling descendants. Other leaf controls cannot contain children. `Element::validate` and `NativeSurface::try_set_root` report invalid trees before replacing the committed UI.

Native toolkit controls retain their platform appearance and keyboard behavior. Tailwind controls the shared layout and common paint properties. Platform control chrome is not pixel-identical to browser controls. Use `accessibility_label` for controls without a visible textual name.

AppKit inherits the system appearance until the host calls `NativeSurface::set_dark_mode`. A host with a fixed light theme should call `set_dark_mode(false)` before mounting. Later calls update existing controls and the appearance used by newly created controls. Native controls own their internal content layout; use a styled `View` around an input for portable outer padding, as the component studio does.

## Vector drawings

Icons, hand-drawn underlines, and chart lines are shapes, not pictures, and a file-based `Image` can't draw them on native platforms. `Vector(view_box)` draws SVG-style paths directly, as native shapes on macOS, iOS, Android, Linux, and Windows, and as inline SVG on the web:

```rust
use leptoskit::prelude::*;

#[component]
fn Underline() -> Element {
	let ink = Color::rgba(0x1e, 0x40, 0xaf, 0xff);
	let stroke = Stroke::new(ink, 3.0).expect("a positive width").round();
	let wave = PathData::parse("M2 6 C20 2 38 9 56 5 S86 2 98 6").expect("valid path data");

	view! {
		<View class="w-[180px] h-[12px]">
			<Vector
				view_box=ViewBox::new(0.0, 0.0, 100.0, 10.0).expect("a positive size")
				scaling=VectorScaling::Stretch
				path=VectorPath::new(wave).stroke(stroke)
				class="absolute inset-0"
			/>
		</View>
	}
}
```

Paths are written in the view box's coordinates and mapped onto the element's frame:

- `VectorScaling::Fit`, the default, scales both axes by the same factor so the whole drawing shows, like SVG's `meet`. Use it for icons and doodles that must keep their shape.
- `VectorScaling::Fill` scales both axes by the same factor so the drawing covers the frame, like `slice`. Add `overflow-hidden` to crop what overflows.
- `VectorScaling::Stretch` scales each axis on its own to fill the frame, like `preserveAspectRatio="none"`. Use it for borders, underlines, and other ink that follows a box of any size.

`.align(VectorAlignment::new(x, y))` places a fitted or filled drawing at the start, centre, or end of each axis, like SVG's `xMinYMax` and its siblings. The element has no size of its own: give it one with classes, or `absolute inset-0` to cover its parent.

### Paths

`PathData::parse` accepts all of SVG's path data: `M`, `L`, `H`, `V`, `C`, `S`, `Q`, `T`, `A`, and `Z`, absolute and relative, with SVG's number syntax and packed arc flags. Elliptical arcs become cubic curves of at most a quarter turn, following the SVG specification's arc rules, including growing radii that are too small to reach the end point. Malformed data is reported with its byte offset.

Basic shapes need no path data: `PathData::rect`, `rounded_rect`, `circle`, `ellipse`, `line`, `polyline`, and `polygon`. `PathData::from_commands` builds a path from absolute `PathCommand`s, for generated geometry such as hand-drawn ink, without formatting a string. `.transform(Affine)` moves, scales, rotates, or skews a path exactly, and `Affine::parse` reads an SVG transform list such as `rotate(45 12 12) translate(2)`.

### Painting

Each `VectorPath` takes an optional `fill(...)` and an optional `stroke(...)`. Both take a `Color` or `Paint::CurrentColor`, which paints with the element's text color, as SVG's `currentColor` does: give the `Vector` a class such as `text-indigo-600`, or let it inherit its parent's, and the icon follows. `.fill_rule(FillRule::Evenodd)` punches holes where shapes nest; the default is SVG's nonzero rule.

`Stroke::new(paint, width)?` has butt caps, miter joins, and a miter limit of 4 unless you set `.cap(...)`, `.join(...)`, `.round()` for both, or `.miter_limit(...)?`. `.dashes(&[4.0, 2.0], offset)?` dashes it, as SVG's `stroke-dasharray` and `stroke-dashoffset`; an odd list repeats, and every dash gets the line's caps. Stroke widths and dashes are in layout units and never scale, so a stretched line stays the same thickness along its length. `.scaling(StrokeScaling::Drawing)` makes them scale with the drawing instead, as SVG strokes do by default; under uneven `Stretch` scaling, native platforms grow them by the square root of the area scale, while the web distorts them with the stretch.

Add several paths with repeated `.path(...)` calls or one `.paths(...)`; later paths paint over earlier ones, and each path paints its fill before its stroke.

### SVG documents

With the `svg` feature, `Drawing::parse_svg(document)` turns an SVG file, such as an icon from your design tool, into a drawing, and `.drawing(...)` puts it on a `Vector`:

```rust
use std::sync::LazyLock;

use leptoskit::prelude::*;

static CHECK: LazyLock<Drawing> = LazyLock::new(|| {
	Drawing::parse_svg(include_str!("check.svg")).expect("the icon imports")
});

#[component]
fn Check() -> Element {
	view! { <Vector drawing=CHECK.clone() class="size-6 text-emerald-600" /> }
}
```

The importer reads `<svg>`, `<g>`, `<path>`, `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, and `<polygon>`, with `viewBox`, `preserveAspectRatio`, nested `transform`s, presentation attributes and `style` declarations, CSS colors (hex, `rgb()`, `hsl()`, and named), `currentColor`, `opacity`, `fill-opacity` and `stroke-opacity`, fill rules, every stroke property, `vector-effect="non-scaling-stroke"`, absolute units, and percentages. `<defs>`, `<title>`, `<desc>`, `<metadata>`, and other tools' namespaces are skipped. Group opacity is folded into each shape's colors, which differs from SVG only where shapes in one translucent group overlap.

Anything else that would change how the document looks is an error that names it, never a silently different drawing: gradients and patterns used as paints, text, `<use>`, images, clipping paths, masks, filters, markers, `<style>` sheets, and font-relative units.

### Performance

Parse a path or import a document once, for example in a `LazyLock`, and reuse it. Paths and drawings are shared, immutable values: cloning one copies a pointer, a screen that reuses the same drawing diffs it with a pointer comparison, and native platforms skip redrawing a drawing that hasn't changed. Path data is formatted for the web and the wire once per path, not once per render.

### Frames, clipping, and accessibility

Drawings are not clipped to their frame, so a stroke on the edge of the view box shows in full. Add `overflow-hidden` or a corner radius to clip. A `Vector` cannot have children. Backgrounds, borders, and opacity apply to it as to a `View`; on the web, a border on the `Vector` itself shrinks the drawing's frame to the inside of the border, while native platforms map the drawing onto the whole frame, so draw outlines as paths instead.

A drawing with an `aria_label` is an image to screen readers. One without a label is decoration, and assistive technology skips it.

## React Native mapping

This table covers all 24 entries in the [React Native core component catalog](https://reactnative.dev/docs/components-and-apis), checked on 20 October 2026. An equivalent names a useful shared contract, rather than full React Native property compatibility.

| React Native entry                                                                              | Leptoskit mapping or missing foundation                                                                                                                                                                                              |
| ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `View`, `Text`, `Image`, `Button`, `ScrollView`, `TextInput`                                    | Shared builders with those names. Text input is single-line.                                                                                                                                                                         |
| `Switch`, `ActivityIndicator`                                                                   | Real shared controls with those names.                                                                                                                                                                                               |
| `Pressable`                                                                                     | `Pressable`, with activation and pressed styles but no press callbacks, as above.                                                                                                                                                    |
| `TouchableHighlight`, `TouchableOpacity`, `TouchableWithoutFeedback`, `TouchableNativeFeedback` | `Pressable` dims while held, as `TouchableOpacity` does, and `active:` utilities restyle it. The separate components and touch lifecycle are not implemented.                                                                        |
| `FlatList`, `SectionList`, `VirtualizedList`                                                    | `VirtualList` in [`leptoskit_virtual`](../guide/17-virtualized-lists.md) renders only the items a scrolled viewport shows. Section headers, sticky headers, and node reuse are not implemented. `ListView` and `GridView` are eager. |
| `Modal`                                                                                         | Deferred. It needs host presentation, focus capture and restoration, dismissal, and accessibility isolation.                                                                                                                         |
| `RefreshControl`                                                                                | Deferred. It needs overscroll gestures and a controlled refresh lifecycle.                                                                                                                                                           |
| `KeyboardAvoidingView`                                                                          | Deferred. It needs host keyboard-occlusion data.                                                                                                                                                                                     |
| `StatusBar`                                                                                     | Deferred host system-UI service.                                                                                                                                                                                                     |
| `InputAccessoryView`                                                                            | Deferred iOS keyboard attachment service.                                                                                                                                                                                            |
| `SafeAreaView`                                                                                  | Deprecated upstream. `*-safe` padding utilities resolve against the host's window insets.                                                                                                                                            |
| `ImageBackground`                                                                               | Deprecated upstream. `Stack` with an `Image` and foreground content supplies the visual composition.                                                                                                                                 |
| `DrawerLayoutAndroid`                                                                           | Deprecated upstream. A portable drawer needs overlays, focus management, and gesture handling.                                                                                                                                       |

## Lynx mapping

Lynx separates its small [built-in element vocabulary](https://lynxjs.org/guide/ui/elements-components.html) from composed UI and extended native elements. Leptoskit uses the same distinction between primitive rendering behavior and optional composition helpers.

| Lynx element                           | Leptoskit contract                                                                                                                                                    |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `view`, `text`, `image`, `scroll-view` | `View`, `Text`, `Image`, and `ScrollView`, with shared Tailwind styling.                                                                                              |
| `page`                                 | The host owns the root view and viewport; no extra page builder is required.                                                                                          |
| `list`                                 | `VirtualList` in [`leptoskit_virtual`](../guide/17-virtualized-lists.md) renders only the visible items. `ScrollView` and the optional `ListView` helper are eager.   |
| `frame`                                | Not implemented.                                                                                                                                                      |
| Extended `input`                       | Single-line `TextInput` and `PasswordInput`; no multiline textarea contract yet.                                                                                      |
| `svg`                                  | `Vector` draws paths and basic shapes with fills, strokes, dashes, and transforms, and imports SVG documents. Gradients, text, clipping, and masks are not supported. |
| Other extended elements                | Overlays, refresh gestures, pagers, blur, embedded web views, and video require additional native contracts. They are not supplied by a styled `View`.                |

## Optional Flutter-inspired helpers

The table groups the [Flutter widget index](https://docs.flutter.dev/reference/widgets) and [widget catalog](https://docs.flutter.dev/ui/widgets) by family and says what leptoskit offers for each.

| Flutter family                                           | Leptoskit coverage and limits                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Basic containers, flex, wrapping, alignment, and sizing  | `Row`, `Column`, `Flex`, `Wrap`, `Padding`, `SizedBox`, `ConstrainedBox`, `AspectRatio`, `Align`, `Center`, `Expanded`, and `Spacer`. Leptoskit uses CSS box sizing and flex allocation.                                                                                                                                                                                                            |
| Overlap and positioning                                  | `Stack` and `Positioned`. No `IndexedStack`, positioned transitions, or directional-inset wrapper.                                                                                                                                                                                                                                                                                                  |
| Painting and decoration                                  | `ColoredBox`, rectangular or rounded clips, `Opacity`, existing backgrounds and uniform borders, and filled or stroked paths through `Vector`. Transforms, shaders, gradients, filters, shadows, and imperative canvas painting need renderer support.                                                                                                                                              |
| Text and images                                          | `Text`, `DefaultTextStyle`, and `Image`. Rich inline text, selection, arbitrary font families, image-provider caches, and animated-image contracts are not implemented.                                                                                                                                                                                                                             |
| Scrolling                                                | Eager `ScrollView`, `ListView`, and `GridView`, and the lazy `VirtualList` from [`leptoskit_virtual`](../guide/17-virtualized-lists.md). Slivers, scroll controllers, snapping pages, nested scroll coordination, and reorderable lists need runtime support.                                                                                                                                       |
| Visibility and hit behavior                              | Collapsing `Visibility`. No offstage measurement, space-preserving visibility, `IgnorePointer`, or `AbsorbPointer`. Disabled controls do not reproduce those hit-test contracts.                                                                                                                                                                                                                    |
| Buttons and selection                                    | `Button`, `Pressable`, `Switch`, `Checkbox`, `Radio`, and `Slider`. No Material or Cupertino theme compatibility, tri-state checkbox, range slider, popup selector, or gesture lifecycle.                                                                                                                                                                                                           |
| Text input and forms                                     | Single-line `TextInput` and `PasswordInput`. Form validation, autofill, IME configuration, multiline editing, focus handles, and selection controllers need additional contracts.                                                                                                                                                                                                                   |
| Progress and communication                               | `LinearProgressIndicator`, `ActivityIndicator`, `Badge`, `Chip`, and dividers. Chips are decorative. Tooltips, snack bars, and banners need presentation and dismissal behavior.                                                                                                                                                                                                                    |
| Cards, list rows, and application layout                 | `Card`, `ListTile`, `AppBar`, and `Scaffold`. No elevation, route ownership, drawers, floating action placement, or automatic safe-area handling.                                                                                                                                                                                                                                                   |
| Navigation, dialogs, sheets, and menus                   | Deferred host presentation, route, focus, and accessibility infrastructure. Static containers do not supply these behaviors.                                                                                                                                                                                                                                                                        |
| Animation and motion                                     | Only platform progress animations. General animation clocks, retained transitions, heroes, and animated layout need a shared runtime.                                                                                                                                                                                                                                                               |
| Gestures, drag, and zoom                                 | Basic control activation. Gesture recognition, drag targets, dismissal, pan, zoom, and reorder behavior need shared input infrastructure.                                                                                                                                                                                                                                                           |
| Accessibility                                            | Every element carries ARIA-shaped props: names, descriptions, roles (including semantic HTML elements on the web), states, range values, live regions, and relationships, mapped onto each platform's accessibility API. Pressables and text inputs require an accessible name; a placeholder is the last resort. Focus scopes, traps, custom actions, and accessibility scaling are still pending. |
| Async, state, and inherited environment                  | Application state and Leptos integration remain application concerns. `FutureBuilder`, `StreamBuilder`, inherited-widget protocols, restoration, and Flutter binding widgets have no direct port.                                                                                                                                                                                                   |
| Styling and platform adaptation                          | Built-in Tailwind, responsive widths, dark mode, inherited text, and safe-area padding. Keyboard metrics, device capabilities, Material themes, and Cupertino themes are not implemented.                                                                                                                                                                                                           |
| Platform views, system services, and development widgets | Deferred host integration. Embedded platform views, platform menus, system shortcuts, inspection widgets, and performance overlays are not shared layout components.                                                                                                                                                                                                                                |

## Backend availability

Web renders semantic DOM controls through Leptos, including server rendering and hydration. macOS renders real AppKit widgets, iOS renders UIKit views, Android renders Android views, Linux renders GTK 4 widgets, and Windows renders Win32 common controls. The headless backend verifies the shared tree and layout without OS widgets.

Every backend crate renders every primitive above, and each is checked in CI on a simulator, an emulator, or a desktop runner. The apps `leptoskit new` generates for macOS, iOS, and Android draw only `View`, `Text`, `Button`, `Pressable`, `TextInput`, `ScrollView`, `Image`, and `Vector` for now. HarmonyOS draws `PasswordInput` but not `Vector`. [Platforms](../platforms/readme.md#what-each-platform-doesnt-do-yet) lists each platform's gaps.

The [CSS coverage reference](css-coverage.md) records engine and utility limits. In particular, the grid engine does not yet implement full right-to-left grid placement. Flex cross-axis alignment does follow its direction.
