# Working on leptoskit

Instructions for coding agents, and a quick orientation for anyone new to the repository. Human contributor workflow is in [`CONTRIBUTING.md`](CONTRIBUTING.md); deeper notes are in [`docs/agents/`](docs/agents/readme.md).

## What this is

leptoskit is a Rust toolkit for writing an app's interface once, as a tree of components (`View`, `Text`, `Button`, …) styled with built-in Tailwind, and rendering it on the web through leptos and as real native widgets on macOS (AppKit), iOS (UIKit), Android, Linux (GTK 4), Windows (Win32), and, experimentally, HarmonyOS (ArkUI). Native apps run the interface as a Wasm guest in an interpreter and replace it over the air with signed code push releases.

## Where things live

| Path                                                    | What it is                                                                                                                                                                                       |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/leptoskit`                                      | The crate apps depend on; re-exports everything behind features.                                                                                                                                 |
| `crates/leptoskit_native`, `crates/leptoskit_yoga`      | Components, the Tailwind compiler, the renderer, and the layout engine.                                                                                                                          |
| `crates/leptoskit_{appkit,uikit,android,gtk,windows}`   | The platform widget backends. AppKit is the reference; compare new behavior against it.                                                                                                          |
| `crates/leptoskit_cli`                                  | The `leptoskit`/`lk` CLI, including the project scaffold (`src/new_project`) and native hosts.                                                                                                   |
| `crates/leptoskit_cli/src/native/templates/harmonyos`   | The experimental ArkUI host; it has no Rust backend crate.                                                                                                                                       |
| `crates/leptoskit_bridge_ffi/{desktop,mobile}`          | The Swift and Kotlin renderers that the generated macOS, iOS, and Android apps use instead of the backend crates. They draw nine widget kinds today; keep the docs' gap lists in step with them. |
| `crates/leptoskit_wasm`, `crates/leptoskit_codepush`    | The on-device Wasm runtime and over-the-air updates.                                                                                                                                             |
| `crates/leptoskit_bridge*`, `crates/leptoskit_plugin_*` | The plugin bridge and the runtime plugins.                                                                                                                                                       |
| `crates/leptoskit_docs`, `docs/site`, `docs/src`        | The docs generator, the site binary, and the public book.                                                                                                                                        |
| `examples/`                                             | The gallery, the platform probe, and example apps.                                                                                                                                               |
| `.github/workflows/`                                    | CI; see [platform validation](docs/agents/platform-validation.md) for what runs where.                                                                                                           |

## Before you change anything

- Run `devenv shell -- true` first so the shell is current, then run commands through `devenv shell <script>` or `devenv shell -- <command>`. Use `CARGO_BUILD_JOBS=4`; full builds are large.
- Formatting, lints, and tests: `devenv shell fix:all`, `devenv shell lint:all`, `devenv shell test:all`. The pre-push hook runs `lint:push`; it is slow enough to outlive git's SSH connection (exit 141), so run the checks yourself first rather than weakening them.
- Generated output is snapshot-tested with insta. Review `.snap` diffs; never accept them blindly.
- Every user-visible change carries a changeset in `.changeset/` (see existing files for the format).
- Configuration changes regenerate the schema with `devenv shell schema:update`; see [CONTRIBUTING.md](CONTRIBUTING.md#configuration-schema).

## Docs

The public site is for app authors. Write it in plain language, start each feature with the problem it solves and a concrete example, and say which platforms an example targets and what happens on each. Repository workflows, CI details, test-file links, and design investigations go in `CONTRIBUTING.md` or `docs/agents/`, never in `docs/src`. [docs/agents/docs-site.md](docs/agents/docs-site.md) lists what the strict docs build rejects.

When you change what a backend supports, update the platform pages and the "Where things stand" section of `docs/src/readme.md` so they keep naming the real gaps per platform.

## Commits and pull requests

- Branch names use conventional prefixes: `feat/`, `fix/`, `docs/`, `test/`, `ci/`, `build/`, `chore/`, `refactor/`.
- Commits are GPG-signed. Never disable or reconfigure signing, and never set repository-local identity or signing config.
- Work through pull requests; never push to `main`. Merge with a squash once every check is green. Dependent changes can go in a `gh stack`.
- Other agents push here too, so `main` moves often. Rebase onto it rather than merging it in.
