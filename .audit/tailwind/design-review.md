# Design cross-judge verdict

Reviewed candidates A and B and the grounding record against current worktree source. The same parent model family produced this review; there is no claim of cross-model diversity.

## Scores

Scale: 1 is weak, 5 is strong. Scores judge the proposed contracts, not an implementation that has not yet been built.

| Criterion                                                |      A |      B |
| -------------------------------------------------------- | -----: | -----: |
| Single source of tree/style truth                        |      5 |      4 |
| Honest native/web behavior, responsive styles and events |      4 |      4 |
| Smallest practical public API and migration cost         |      5 |      3 |
| Strict supported-feature diagnostics                     |      4 |      5 |
| Testability/extensibility without a framework fork       |      4 |      4 |
| Total                                                    | **22** | **20** |

## Recommendation

Use **A as the base**, with the diagnostic/capability discipline of B. A gets the required cross-platform screen API by extending the actual existing element model rather than asking every existing native screen to adopt a new Node model, six wrapper types, and a lowering layer simultaneously. Capitalized constructors are a reasonable first public built-in component API if the docs explicitly call them Rust builder components and show actual web/native use. They must not be presented as `<View>` Leptos components.

B's private typed primitive bodies prevent invalid child combinations and keep native internals out of application code. That is a real benefit, but the closed shared model also omits existing TextRun and Custom kinds and needs decisions about backend extension interoperability. It would remain one _authoritative application_ tree, rather than a second reconciliation engine; A's critique overstates that risk. The objection is the extra semantic/lowering representation and migration required before the user's requested features, not an unavoidable duplicate reconciler. Shape safety alone does not justify changing the entire tree contract in this feature.

A's public base style/props plus class program are not automatically competing style truths: explicit, documented class-over-base precedence and pure resolution make that model coherent. The implementation must never mutate the source base with resolved values, or responsive re-resolution will retain stale declarations.

## Required grafts and amendments

1. **Take B's whole-tree capability preflight and transactional failure behavior.** Add a fallible native commit path; validate the complete proposed tree, collect errors with token range and component path, then mutate. Leave the old tree and viewport/environment unchanged on a rejected commit. A's static `.class()` convenience can remain a documented panic-on-programmer-error wrapper over `.try_class()`, but runtime/untrusted and target validation must have Result APIs.
2. **Separate syntax support from target capability.** Both designs have the right single utility registry. Generate the public matrix from it and use the same records for parse and commit diagnostics. An unimplemented class errors everywhere. A recognized property unavailable on a target yields a precise capability error there. No browser-only escape hatch in the shared primitive contract. Unknown Custom widgets must also fail on the web unless a renderer is registered.
3. **Take B's explicit environment vocabulary**, especially writing direction, root font size, color scheme, and interaction state. Avoid hover/focus/active utilities until events actually change native state and trigger re-resolution. Dark mode requires platform-shell updates, not merely a setter tested manually.
4. **Retain A's detailed acceptance tests**, including pinned official Tailwind CSS fixtures, browser geometry comparisons, a real mounted click/signal update, resize across a breakpoint, text inheritance and text measurement changes, SSR/CSR determinism, and repeated-environment idempotence. Add B's invalid-tree rollback test. A web renderer that compiles and headless tests alone do not prove a shared UI platform.
5. **Keep primitive contracts small but real.** Add explicit accessible image description/input label and controlled input/event methods to the common Element builder API, rather than forcing examples to write generic NativeProps. Validate children where the renderer cannot represent them. Do not add six wrapper structs solely for naming. Existing raw native constructors remain available for backend implementors.
6. **Take A's source-root retention and responsive ownership model.** NativeSurface owns the unresolved source and resolves it once through one function on root/environment changes; web emits media rules from the same declarations. No global viewport state and no per-class side table keyed by transient node IDs.
7. **Do not advertise unsupported CSS.** Keep both candidates' honest scope language. CSS full parity is browser-engine-sized. Support can grow substantially without claiming block/fixed/baseline/scroll/effects work before end-to-end proofs exist.

## Source-grounded implementation blockers

Paths below are relative to `/Users/ifiokjr/.codex/worktrees/tailwind-primitives/leptoskit`.

- **No native Leptos view bridge exists.** `crates/leptoskit/Cargo.toml:19` has no Leptos dependency, and the installed `tachys-0.2.19/src/renderer/mod.rs:11-20` explicitly says generic rendering was removed and hardcodes `Rndr = dom::Dom`. Both designs correctly avoid claiming a small native `IntoView` adapter. Any implementation that quietly substitutes a web-only `<View>` is a blocker.
- **Responsive state currently cannot be re-resolved.** `crates/leptoskit_native/src/surface.rs:63-70` stores only committed shadow/Yoga state; `:101-107` merely runs layout on resize, and `:136-145` immediately mutates/commits the supplied Element. Source retention and preflight need actual changes here. Parsing `md:` then resolving only at initial construction fails the contract.
- **Web event access needs a deliberate boundary.** `crates/leptoskit_native/src/element.rs:32` keeps handlers private; the accessor near the end of that file is `pub(crate)`. A separate web crate cannot use it. Supply a small event-dispatch/access contract, rather than making HandlerEntry's internals broadly public or copying callbacks into another global registry. Existing native bubbling is in `surface.rs:147-163` and should define the shared behavior.
- **Layout and visual borders currently diverge.** `crates/leptoskit_native/src/element.rs:178-186` sets paint only; `props.rs:221-228` explicitly documents layout separately. One declaration must derive both. Advertising border utilities by setting only NativeProps would fail geometry parity.
- **There are known CSS behavior defects to gate utilities on.** `crates/leptoskit_yoga/src/flexbox.rs:153` combines reverse/RTL with OR; `style.rs:126-128` explicitly documents Fixed as parent-relative Absolute; `grid.rs:442-450` treats minmax fractional maxima as content sizing while `:533-550` distributes space only to top-level Fraction tracks. Grid-cols-N and fixed cannot be called correct before these are repaired or rejected.
- **Overflow/hit testing is not presently browser-equivalent.** `crates/leptoskit_native/src/surface.rs:188-198` descends into children only when the parent contains the point, independent of overflow style. Thus overflow-visible descendants cannot receive outside-parent hits. NativeProps (`props.rs:199-242`) has no overflow delivery field. Add paint/backend/interaction semantics together before class acceptance.
- **Typed B drops extension cases unless amended.** `props.rs:16-32` includes TextRun and Custom, while B's closed Primitive omits them. A migration must preserve intentional rich text/backend widget use or explicitly reject it with a documented path; it cannot silently lower every node to the six basic kinds.

## First proving slice

Before broad utility expansion: one shared counter tree rendered through native/headless and mounted Leptos DOM; `p-4 md:flex-row border`, inherited text style, an accessible enabled button with a real click callback, and an invalid class/capability rejection that preserves the committed tree. Resize in both targets. This slice exercises the highest-risk architectural promises; only then spread the registry and migrate gallery/templates/examples.
