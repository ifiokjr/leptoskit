# Native widget ownership and verification

Applications keep returning an `Element` tree. The default imports emphasize real primitive kinds: containers, text, images, scrolling, input, buttons/pressables, selection controls, and progress. Flutter-inspired layout/presentation constructors remain available under an explicitly imported helpers module. A stack is ordinary styled `View` composition, not a platform widget.

## Existing shared contract

`NativeSurface<B>` owns source descriptions, validation, Tailwind resolution, reconciliation, and Yoga. `NativeBackend` consumes ordered `MutationList` batches and supplies a `Send + Sync` text measurer. Native widget references stay owned by the creating UI thread. Application callbacks run outside borrowed backend state. Programmatic property updates and teardown suppress native input notifications; native user proposals restore the latest application-owned value after callbacks, even if a callback replaces or removes the firing widget.

This ownership seam is already settled and verified in AppKit. Replacing it with a second cross-platform tree or a generic service bridge would duplicate ownership. The implementation adds target-gated backend crates with the same contract. The existing native plugin ABI remains separate.

## Platform boundaries

| Platform | Backend crate       | Concrete UI                                       | Verification                                                                     |
| -------- | ------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------- |
| iOS      | `leptoskit_uikit`   | UIKit views, controls, and scroll document        | Actual simulator app, native actions/editor/scroll assertions, pixels            |
| Android  | `leptoskit_android` | Android Views with a dedicated JNI widget adapter | Actual task-owned emulator Activity, native actions/lifecycle assertions, pixels |
| Windows  | `leptoskit_windows` | Win32 windows and native controls                 | Hosted Windows HWND/keyboard/accessibility tests and screenshot artifact         |
| Linux    | `leptoskit_gtk`     | GTK4 controls and an exact outer frame/container  | Real GTK window under Xvfb, allocations/input/accessibility assertions, pixels   |

Each backend covers all 15 existing built-in `NativeKind` values, including internal `TextRun`. The 45 current public builders compose those kinds, so helpers require no new backend widget classes. New public API work follows React Native and Lynx primitive behavior; this task adds no Flutter composition types.

Native `UISlider`, Android `SeekBar`, and Windows trackbars cannot directly represent arbitrary shared `f64` slider grids. Their coordinates must normalize into the existing `SliderState` domain and canonical proposal rules. Native interior sizing cannot redefine Yoga outer frames. GTK minimum-size behavior requires an explicit outer frame implementation and allocation assertions. Styling includes colors, borders, corner radii, clipping, opacity, font metrics, visibility, and disabled ancestry, rather than layout coordinates alone.

Yoga coordinates are logical layout units. Each host supplies logical content bounds again on resize, rotation, or DPI changes; physical-pixel conversion happens only at the platform boundary. Text measurers use immutable matching font metrics for a surface, because layout caches do not invalidate themselves for arbitrary platform environment changes. After changing that environment, hosts call `NativeSurface::refresh_text_measurer()` to replace snapshots and reflow existing text without recreating controls. Resolved native props and text styles carry each node's Yoga writing direction for shaping and logical alignment.

Native callbacks clone their listener and release all collection borrows before application dispatch. Firing targets remain retained through callbacks that rebuild or remove them. Teardown invalidates the owner first; late queued events must check active owner/generation state before dispatch. Effective disabled and hidden ancestry controls focus, native editing, accessibility activation, and animation, not just shared event rejection.

The supported shared image contract is local files and file URLs, plus explicit packaged-resource support where the host provides it. This does not add implicit network fetching.

## Integration ownership

Platform workers own their new crate, native adapter sources, and focused native verification binaries. Root owns workspace dependency reconciliation, the public facade, optional-helper imports, gallery entrypoints, generated project integration, docs, and CI workflows. Every platform worker uses a separate external worktree. Integration retains the current AppKit and web behavior.

Real renderer verdicts are VERIFIED, NOT VERIFIED, or INCONCLUSIVE. Cross-compilation and headless journeys supplement native execution and cannot substitute for it. Reviewers independently check control rejection, reentrant removal, hidden/disabled ancestors, native naming and roles, appearance, text metrics, scroll extents/events, loading animation shutdown, and resource ownership.
