# Tailwind and shared primitives

- [x] Read project and skill constraints, use a separate worktree from origin/main.
- [x] Ground layout, rendering, web integration, and spec coverage.
- [x] Compare two independent API designs and record chosen interfaces.
- [x] Correct supported CSS layout semantics with coordinate-level regression tests.
- [x] Add built-in Tailwind compiler with explicit unsupported-class diagnostics.
- [x] Add shared View/Text/Image/ScrollView/TextInput/Button primitives and web rendering.
- [ ] Migrate examples, demos, gallery, and generated starter to shared primitives.
- [ ] Verify native/headless and web behavior, inspect actual rendered output.
- [ ] Format, lint, test workspace, document CSS/Tailwind coverage and limits.
- [ ] Open pull request(s), independently review, resolve CI/review, merge, clean up.

Done requires shared component use in every first-party demo, class-driven layout and native appearance, equivalent web output, tested updates and input/events, no silent acceptance of unsupported CSS, required checks passing and merged PRs. CSS is modular; this work must distinguish implemented layout from rendering, selectors, text, interaction, and browser-only capabilities and cannot label an unsupported module as complete.
