# Flutter widget-index inventory

Snapshot: 2026-10-20, retrieved by 11:49 UTC. Source: [Flutter alphabetical widget index](https://docs.flutter.dev/reference/widgets). The page identifies its documentation as Flutter 3.47, last updated 2026-09-29.

The companion [`flutter-widgets.tsv`](flutter-widgets.tsv) contains **all 229 card entries** in source order, with the exact displayed name and linked upstream URL. Counts by linked library are 115 Flutter SDK entries, 53 Material entries, and 61 Cupertino entries. These counts include non-widget services, data types, controllers, state classes and grouped catalog entries when the index itself includes them. They are not a count of every class in the Flutter SDK.

The index is curated. For example, it does not list `Flex`, `Spacer`, `Positioned`, `ColoredBox`, `ClipRRect`, `Visibility` or `SafeArea` as cards in this snapshot. This audit does not invent rows for absent entries or claim that indexing this page enumerates the entire API reference. Leptoskit's planned additions include useful builders absent from this particular index.

## Status meanings

| Status        | Entries | Meaning                                                                                                                                                                                                                      |
| ------------- | ------: | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `implemented` |       4 | The shared implementation provides the named core behavior, within the per-row contract. This does not mean every Flutter property, visual specification or platform variant is reproduced.                                  |
| `partial`     |      73 | A named Leptoskit component or existing shared primitive supplies a useful reduced behavior. The row identifies missing behavior, differing layout rules or design-system scope. Several rows can map to the same component. |
| `deferred`    |     152 | There is no implementation of the widget or service contract. Related primitives may be named as building blocks, but are not counted as an implementation.                                                                  |

The 77 implemented/partial rows are **upstream entries with mappings**, not 77 new Leptoskit components. This change plans 31 layout/presentation builders, seven real controls, and activation-only `Pressable`, in addition to the existing six shared builders. Entries are conservatively classified against those contracts rather than treated as API-compatible Flutter clones.

Widget rendering claims apply to the implemented AppKit and web adapters. Headless Yoga or WASM tests do not establish rendering support for an unfinished platform backend.

## Implementation evidence

The mapping was grounded in the actual worker sources for this change:

- `crates/leptoskit_native/src/components/layout.rs`: 31 builders. CSS flex/grid and box-model contracts, explicit sizing/constraints, nine-position alignment, direct-child expansion, grid-cell overlap, clipping/opacity, limited typography inheritance, eager lists/grids and neutral presentation slots. Unlike Flutter, these builders do not implement its parent-to-child constraint protocol.
- `crates/leptoskit_native/src/components/controls.rs` and `src/controls.rs`: `Switch`, boolean `Checkbox`, grouped `Radio`, validated horizontal `Slider`, determinate/indeterminate `LinearProgressIndicator`, animated `ActivityIndicator`, and secure `PasswordInput`. Application state remains authoritative; events propose typed changes. Checkbox mixed state and determinate circular progress are absent.
- `Pressable`: accessible primary activation with decorative child content, actual native/web button behavior, a required accessible label, and no nested interactive descendants. It does not supply long-press, pointer-lifecycle or arbitrary gesture detection.
- Existing `components.rs`, `element.rs`, `props.rs`, `surface.rs`, `web/mod.rs`, `web/styles.rs`, Yoga `style.rs`, and AppKit `backend.rs`/`views.rs`: source representation, styling precedence, event/hit-test boundaries and available backend behavior.

At inventory creation the worker files had not all been integrated into the root worktree. These statuses describe the implementation contracts being delivered by this change; they do not assert completed final verification. Root integration, review and runtime/CI evidence remain the shipping gate.

## Material and Cupertino

The [current category catalog](https://docs.flutter.dev/ui/widgets) describes a neutral SDK widgets library with Material and Cupertino design packages above it. The [Material](https://docs.flutter.dev/ui/widgets/material) and [Cupertino](https://docs.flutter.dev/ui/widgets/cupertino) catalogs link to their package documentation. This inventory preserves those actual URLs, including public display labels such as `Bottom app bar`, `Common buttons`, and the index spelling `CupertinoCheckBox`.

Neutral Leptoskit controls and layouts are useful mappings for several variants, but they do not establish Material/Cupertino visual, motion, theme or platform-service parity. For example, `Card` has no elevation, `Chip` is decorative, `ListTile` is noninteractive, `Scaffold` only supplies header/body/footer layout, and `ActivityIndicator` is indeterminate.

## Uncovered catalog areas

- General animation controllers, implicit animations, transitions, Hero flights and animated collection lifecycle. Specialized progress animations do not provide this runtime. [Animation catalog](https://docs.flutter.dev/ui/widgets/animation).
- Lazy lists, slivers, coordinated nested scrolling, refresh/reorder gestures, paging, scroll controllers and physics. `ListView` and `GridView` are eager. [Scrolling catalog](https://docs.flutter.dev/ui/widgets/scrolling).
- Routing, page/tab retention, overlay/modal hosting, focus trapping/return, anchored menus, dialogs, drawers, tooltips and transient message lifecycle. [Interaction catalog](https://docs.flutter.dev/ui/widgets/interaction).
- Parent-constraint builders, layout delegates, intrinsic-pass widgets, baseline alignment, loose flexible allocation, exact offstage behavior and specialized overflow algorithms. Existing CSS layout is not an alias for these contracts. [Layout catalog](https://docs.flutter.dev/ui/widgets/layout).
- Arbitrary canvas/path painting, oval/path masks, transforms, filters and full decoration. Rounded rectangle clipping does not implement an oval or arbitrary path. [Painting catalog](https://docs.flutter.dev/ui/widgets/painting).
- Arbitrary semantics trees, merge/exclude boundaries, actions and traversal; role/state on supported controls covers only part of accessibility. [Accessibility catalog](https://docs.flutter.dev/ui/widgets/accessibility).
- Rich inline text shaping, advanced editing/selection, text magnifiers/toolbars/spellcheck, form registration/validation, autocomplete, date/time/wheel pickers, and general key/gesture recognition. [Text](https://docs.flutter.dev/ui/widgets/text), [input](https://docs.flutter.dev/ui/widgets/input).
- Full device/environment data, keyboard/safe-area insets, localization, asset bundles, dynamic design-system themes, async snapshot builders and table semantics. Some entries are framework services rather than visual widgets that should be copied into a component namespace. [Styling](https://docs.flutter.dev/ui/widgets/styling), [assets](https://docs.flutter.dev/ui/widgets/assets), [async](https://docs.flutter.dev/ui/widgets/async).

## Extraction and verification

The official HTML was fetched directly. A small standalone Rust extractor selected every `a.card.outlined-card`, removed only formatting tags such as `<wbr/>` from the displayed card title, and retained its exact `href`. Exact `(name, URL)` pairs were deduplicated; this snapshot had no duplicates. Non-widget entries were retained. A separate exhaustive classifier required an explicit classification or a narrowly defined animation-name rule for every extracted entry and failed on an unclassified name.

Checks: 229 HTML `card-title` elements, 229 extracted exact pairs, and 229 data rows with six nonempty TSV fields; all source name/URL pairs are preserved in order. The source HTML SHA-256 is `a77da882f15d226c86251395f78b70ec47508dbc652383e1828048414f38e857`.

The source HTML and temporary Rust extraction tools were kept outside the repository. Only this companion note and the TSV belong to this audit. Re-fetch the index when refreshing it; `latest` package links and catalog membership can change.
