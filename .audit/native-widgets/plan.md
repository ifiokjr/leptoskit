# Native widget backends

- [x] Read the Poteto principles and applicable Git, Rust, coding-style, and devenv instructions.
- [x] Ground the shared mutation, ownership, and host contracts and each platform's real UI facilities.
- [x] Record the definition of done, verification matrix, and independent implementation boundaries.
- [x] Establish the unchanged workspace baseline and real native verification harnesses.
- [ ] Implement and verify UIKit rendering and a real iOS application host.
- [ ] Implement and verify Android rendering and a real Activity host.
- [ ] Implement and verify Windows rendering and a real desktop host.
- [ ] Implement and verify Linux rendering and a real desktop host.
- [ ] Integrate native backend selection, the shared gallery, generated hosts, and platform CI.
- [x] Keep Flutter-inspired compositions as optional helpers, with React Native/Lynx-style primitives as the primary surface.
- [ ] Independently review ownership, controlled input, accessibility, rendering, and actual platform evidence.
- [ ] Run required formatting, static checks, workspace/browser tests, and real platform CI.
- [ ] Deliver reviewed PRs through green checks and merge; clean up owned worktrees.

Done means the same shared primitive tree mounts real OS views on macOS, iOS, Android, Windows, Linux, and the web. New backends must consume the existing Yoga mutation stream and render the current built-in widget kinds, including text, images, scrolling, editable controls, buttons/pressables, selection, sliders, and progress. Accepted and rejected controlled proposals, updates without identity loss, native keyboard/accessibility activation, appearance, and teardown must be exercised against actual platform objects. A headless probe or successful cross-compilation alone is not a passing renderer verdict.

The existing `Element`, `NativeSurface`, and `NativeBackend` ownership model remains the shared representation. Composition helpers do not require new native widget types. The user chose to retain Flutter-inspired helpers as optional imports while focusing new work and examples on React Native and Lynx primitives.

This is a high-rigor four-platform implementation. Each platform implementation and its tests live in an isolated worktree. Root owns shared integration and CI. Independent review is required before shipping. Available agent models are all GPT-family, so review does not claim model-family diversity.
