# Prompt: port the leptos examples to leptoskit, every platform included

> Working title for the agent: **"Port the leptos examples to leptoskit"**
>
> This document is the full brief. Do the work on a branch named
> `feat/port-leptos-examples`, in its own worktree, and open a pull request —
> never merge to `main` directly. Merge only when every CI check passes.

## Goal

The upstream [leptos repo](https://github.com/leptos-rs/leptos) ships a
`examples/` directory of small apps (todo_app_sqlite, todo_app_sqlite_csr,
todo_app_sqlite_axum, hackernews, hackernews_axum, session_auth_axum,
tailwind_actix, tailwind_axum, error_boundary, counter, counter_isomorphic,
crater, fetch, globals_async, parent_child, router, slots, stable_counter,
ssr_modes, suspense, errors, …). Port the majority of them to leptoskit so
that leptoskit has a comparable gallery — **and every ported example must run
on every platform leptoskit supports**: the web build, the server, and the
native targets — macOS, iOS, Android, Windows, and Linux — including their
Android and iOS shells.

"Majority" means: every example whose purpose still communicates on leptoskit
(counters, routing, suspense, fetch, globals, slots, hackernews, todo apps,
auth/session, tailwind styling, error boundaries). Skip only what is
meaningless here (examples that exist to test a leptos-internal macro edge
case with no user-visible behavior, or that need an actix-only stack).

## Non-negotiables

1. **Nothing breaks.** `cargo clippy --workspace --all-features --all-targets
   -- -D warnings`, `cargo nextest run --workspace --all-features`, the doc
   tests, `dprint check`, `cargo deny check`, and `monochange check` all stay
   green. Every existing example (counter, gallery, islands, platform_probe,
   plugin_guest) keeps building.
2. **Every ported example builds for every platform.** The proof is per
   example: `leptoskit build --platform web` and `--platform server` succeed;
   the native shells at least compile for the host (`--platform native`), and
   the Android and iOS host files exist and are wired the way `examples/
   gallery` wires them. Where a platform genuinely cannot run an example
   (say, an example about SSR hydration islands on a headless Android
   surface), the example's readme says so explicitly and the build still
   succeeds for that platform by shipping the closest equivalent screen —
   do not `cfg` an example out of existence.
3. **Each port is recognizable.** Keep the upstream example's name
   (`hackernews`, `todo_app_sqlite`, …), its screens, and its purpose. Port
   the *idea*, not line-by-line code: idiomatic leptoskit uses `view!` /
   components, typed styles or Tailwind classes the way `examples/gallery`
   does, `leptoskit_fns` for anything that talks to a server, and the file
   router when the example is about routing.
4. **Examples are publish-free workspace members** (like the current ones:
   `publish = false`, workspace lints). Each carries a `readme.md` saying what
   it demonstrates and how to run it per platform.
5. **A changeset is required** (the repo's `monochange` flow fails a PR
   without one for changed packages; examples are ignored paths, so add one
   only if you touch a crate).

## How to port one example

Work incrementally: one example per commit, in this order (each is a
self-contained lesson that later ones reuse):

1. **counter_isomorphic** — the smallest full-stack app; introduces the
   scaffold: `Cargo.toml` with `ssr`/`hydrate`/`csr` feature wiring, a
   `client` crate, `public/index.html`, and the `leptoskit.toml`. Verify with
   `leptoskit build` and by opening `dist/index.html`.
2. **fetch** — data fetching through `leptoskit_fns::query` with
   `RetryPolicy`, showing `FnError` classification in the UI.
3. **router** and **slots** — leptoskit's file router; port the nested-routes
   and slots lessons. If the file router covers less than the example needs,
   stop and report the gap instead of forking the router.
4. **suspense**, **error_boundary**, **globals_async**, **parent_child** —
   reactive primitives and boundaries; these are mostly client-side, so their
   native story is the interesting part.
5. **hackernews** (and **hackernews_axum** merged into one port) — the
   flagship: listing, item pages, pagination, users, SSR vs CSR modes. Use
   `leptoskit_fns` server functions behind `#[api]` where the example talks
   to its data source, and generate nothing (the example *is* the client).
6. **todo_app_sqlite** (with `_csr` and `_axum` folded in as modes) —
   persistence, mutations, `leptoskit_fns::mutation`, and `TodoFilter`-style
   schemas. On native platforms without a server, ship an in-memory
   repository behind the same server functions so the screens still run.
7. **session_auth_axum** — cookies/session middleware on the server side,
   login/logout screens everywhere.
8. **tailwind_axum** (tailwind_actix folds into it) — styled with Tailwind
   through leptoskit's shared component styling, verified on web and at least
   one native host screenshot.
9. **stable_counter, errors, crater, counters_isomorphic** if time allows —
   small ones, quick ports.

For each: create `examples/<name>/`, copy the structural pattern from
`examples/islands` (it has `leptoskit.toml`, `public/`, a `build.rs`, and
feature wiring — the cleanest minimal template) and the multi-platform
pattern from `examples/gallery` (it has `src/bin/web.rs`, `src/bin/ios.rs`,
`android/` with `AndroidManifest.xml` + Kotlin activity, `ios/` with
`Info.plist`, and a `verify/` harness). Add the example to the workspace
members if the glob does not already catch it (`examples/*` is a workspace
member pattern — check the root `Cargo.toml`).

### Server functions in examples

Use `#[leptoskit_fns::api]` above `#[server]` for every server function an
example defines, with a doc comment and, where arguments benefit, an
`# Arguments` section — the examples double as documentation of the RPC
publishing flow. Do not wire `leptoskit fns generate` into their CI; the
readme may mention it as an exercise.

### Android and iOS

Copy `examples/gallery/android` and `examples/gallery/ios` as the starting
point (rename the activity, the package id under
`leptoskit.toml [native] package_id`, and the bundle id). The example must
carry its own `leptoskit.toml` `[native]` section listing the targets it
actually ships. If the host shells need a widget the renderer does not expose
yet, **stop and report** — do not hand-roll native widgets in an example.

### Verification per example (put this in each readme)

```bash
leptoskit build --platform web --out dist   # and open dist/index.html
leptoskit build --platform server
leptoskit build --platform native           # compiles the host for this machine
```

plus, for examples with tests, a `tests/` harness in the style of
`examples/gallery/tests` (host-driven) or `examples/platform_probe/tests`.
Where screenshots communicate (Tailwind, hackernews, todos), add them to
`docs/src/assets/<example>/` and reference from the readme — the docs site
already pulls from `docs/src/assets`.

## Definition of done

- At least 12 ported examples (the list above), each building for web,
  server, and native, with Android and iOS host files present.
- `docs/src/examples.md` updated: a section per example, one screenshot for
  the visual ones, and a table of which platforms each runs on.
- The repo's `readme.md` examples paragraph mentions the ported gallery.
- All CI green on the PR; `monochange check` passes; dprint formatted.
- No regressions to existing examples or crates.

## Reporting discipline

At the end of every work session, leave a note in the PR description (or a
`docs/porting-progress.md` scratch file you delete before merging) listing:
ported examples, examples skipped with reasons, platform gaps found in
leptoskit itself (router coverage, missing widgets, missing server
integrations) — those gaps are the most valuable output of this port, so
record them precisely rather than working around them silently.
