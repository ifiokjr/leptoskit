---
"release": minor
---

# Add ARIA accessibility across every platform

Every element now carries ARIA-shaped props — accessible names and descriptions (with id references), roles, states, range values, live regions, modal flags, set positions, orientation, tab order, and a passthrough for any other `aria-*` attribute — rendered as real attributes and semantic HTML elements on the web (headings, lists, landmarks, `<hr>` separators) and mapped onto each platform's accessibility API: VoiceOver traits on iOS, NSAccessibility roles and announcements on macOS, TalkBack class names and live regions on Android, MSAA roles with UIA control types and notifications on Windows, and GTK accessible properties, states, and relations.

Built-in components carry their semantics: Heading, Status, and Alert join the vocabulary; dividers are separators; lists, tiles, the app bar banner, and the scaffold main landmark announce their structure; activity indicators are named; and controls with a declared control role join the tab order and activate from Enter and Space. Validation rejects nameless text inputs and pressables, duplicate or dangling relationship ids, named presentation, and passthrough attributes that shadow typed builders.

The widget wire stays additive: `accessibility_label` keeps its key, unknown roles decode to none, and the resolver folds labelled-by and described-by references into concrete labels for native backends, numbers radio options within their group, and fills live-region announcement text. The widget tester gains `Finder::aria_role` and a semantics tree dump; the AppKit, browser, and widget suites cover the mapping, and the gallery demonstrates headings and a typing-indicator status region.

Focus scopes, traps, and custom accessibility actions remain pending; runtime services such as announcements outside live regions, screen-reader detection, and reduce-motion belong in a follow-up accessibility runtime plugin.
