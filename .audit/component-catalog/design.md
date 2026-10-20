# Component contracts

The existing `Element` tree remains the shared representation. Builders set typed base styles/props and return `Element`; `.class` remains authoritative for styling. Native rendering resolves retained source descriptions, reconciles, runs Yoga, and applies backend mutations. The web adapter emits real Leptos DOM nodes with scoped CSS. No extra reactive component runtime is introduced.

Portable compositions cover axes/wrapping/grid, explicit sizing and constraints, alignment/padding/expansion, overlapping stacks, clipping/opacity/text inheritance, eager scrolling collections, and common neutral surfaces and layout slots. Constructors document their Leptoskit contract and differences from Flutter. In particular, no eager list is called FlatList, no grow wrapper is called Flexible.loose, no hidden wrapper is called Offstage, and no absolute container is called Modal.

Real controls extend NativeKind/NativeProps/NativeEvent, AppKit and the optional web adapter together. Boolean and numeric state changes use typed events rather than text strings. Slider values/ranges parse into a validated shared type. Controlled widgets propose updates; application state remains authoritative, including when an owner rejects an update. Radio grouping is explicit. Progress exposes value/loading semantics and animation lifecycle. Fixed constructor defaults keep controls measurable while Tailwind overrides their layout.

The AppKit backend owns native objects and retains weak action/delegate targets. The web adapter owns kind-specific semantic elements and controlled properties; typed events must not become string Change events on DOM ancestors. WASM numeric codes are appended, existing codes retained, and invalid external numeric state is rejected before commit. WidgetTester uses the same typed proposal events and documents its simulated-input boundary.

Rich Pressable is conditional on a correct native button container with flipped coordinates and decorative-child hit routing. It must support keyboard activation and accessible naming, and prohibit nested interactive descendants. A plain View with a click listener does not qualify.

The catalog audit covers all 24 current React Native component entries and Flutter's published widget families, with explicit groupings for visual widgets, framework plumbing, design-system variants, and missing platform services. AppKit and web are implemented widget renderers; other native backends remain an explicit gap.
