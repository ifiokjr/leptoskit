# Formatting

rustfmt formats Rust, but it leaves the markup inside `view!` alone. `leptoskit format` does both: rustfmt formats the Rust, with the project's own `rustfmt.toml`, and leptoskit lays out the markup at the same width and indentation. Every Rust expression inside the markup, from an `on:click` closure to a `{move || …}` child, goes back through rustfmt at the column it sits in.

```bash
leptoskit format                  # every Rust file under the current directory
leptoskit format src/app.rs       # one file, or any list of files and directories
leptoskit format --check          # write nothing; print a diff and fail if anything would change
leptoskit format --stdin --stdin-filepath src/app.rs < src/app.rs   # for editors
```

`lk format` is the same command. Directories are walked with `.gitignore` respected, and `--exclude '**/fixtures/**'` skips more. `--check` fails when any file would change, so it fits in CI as it is.

## The style

```rust,ignore
#[component]
pub fn TodoList(todos: ReadSignal<Vec<Todo>>) -> impl IntoView {
	let (draft, set_draft) = signal(String::new());
	view! {
		<section class="todos">
			<h1>"Todos"</h1>
			<input
				class="new-todo"
				placeholder="What needs to be done?"
				prop:value=draft
				on:input=move |ev| set_draft.set(event_target_value(&ev))
			/>
			<For each=move || todos.get() key=|todo| todo.id let:todo>
				<li class:done=move || todo.done.get()>{todo.title.clone()}</li>
			</For>
			<p>"Remaining: "{move || remaining(&todos)}" of "{move || todos.with(Vec::len)}</p>
		</section>
	}
}
```

- **A macro with one node stays on one line** when it fits: `view! { <Counter initial=3 /> }`. Several nodes get a line each.
- **Children that start with text or a `{block}` stay inline** when they fit, and otherwise pack onto as few lines as fit, like words in a paragraph. Any other children get a line each.
- **One attribute never wraps.** Several share the tag's line when they fit; otherwise each gets a line, and the closing `>` or `/>` gets its own.
- **Braces come off attribute values the parser does not need them for**: `class={name}` becomes `class=name`, while `title={a > b}` keeps them, since an unbraced `>` would end the tag. Braces around a parenthesized value stay exactly as written: leptoskit's `view!` spreads `margin=(1, 2)` into two arguments but passes `margin={(1, 2)}` as one tuple.
- **Elements without children self-close**: `<div></div>` becomes `<div />`. HTML void elements such as `<br />` always do.
- **Comments stay where they were** relative to their neighbours, before or after a node or an attribute, and a run of blank lines between children becomes one.

What the formatter never changes:

- Text and string literals, raw strings included, are copied byte for byte, even across lines. Unquoted text is copied as written.
- Markup that does not parse is left exactly as written, with a warning naming the line. So is a comment somewhere a layout cannot keep it, such as inside a tag name.
- Items marked `#[rustfmt::skip]` are left alone, markup and all.

Formatting is idempotent: a formatted file formats to itself.

## Configuration

Width and indentation come from `rustfmt.toml` (or `.rustfmt.toml`), found the way rustfmt finds it, so the markup and the Rust around it always agree: `max_width`, `hard_tabs`, `tab_spaces`, `newline_style`, and the edition. The rest of the markup style lives in the `[format]` table of `leptoskit.toml`, and every key is optional:

```toml
[format]
# The macros whose markup is formatted, matched against the path written at
# the call site: `view` matches `view! {}` but not `leptos::view! {}`.
macros = [
	"view",
	"leptos::view",
	"leptos::prelude::view",
	"leptoskit::view",
	"leptoskit::prelude::view",
	"leptoskit_macros::view",
]
# "when_required", "always", "always_unless_literal", or "preserve".
attribute_braces = "when_required"
# "self_closing", "explicit", or "preserve".
closing_tags = "self_closing"
# The rustup toolchain whose rustfmt runs: nightly, so unstable rustfmt.toml
# options apply, unless set to "stable" or another toolchain.
rustfmt_toolchain = "nightly"
```

`--stable` or `--toolchain <name>` overrides `rustfmt_toolchain` for one run, and the `LEPTOSKIT_RUSTFMT` environment variable names a rustfmt binary to use instead of any toolchain's.

## Editors

Point rust-analyzer's formatter at the command, in VS Code's `settings.json`:

```json
{
	"rust-analyzer.rustfmt.overrideCommand": ["leptoskit", "format", "--stdin"]
}
```

or in Helix's `languages.toml`:

```toml
[[language]]
name = "rust"
auto-format = true
formatter = { command = "leptoskit", args = ["format", "--stdin"] }
```

With `--stdin`, configuration is found from `--stdin-filepath` when given, and from the working directory otherwise.

## How it is fast

rustfmt does most of the work, and starting it costs tens of milliseconds, so the formatter starts it as rarely as it can:

- **One rustfmt run per file.** Every markup macro is masked before rustfmt sees the file, since rustfmt re-indents the lines of macro bodies it cannot parse, including the inside of multi-line string literals. The expressions inside the markup become functions appended to the masked file, each nested to the indentation it will have and bound to a name as wide as the text before it, so one rustfmt run formats the file and every expression against the width it really has. Formatting a single file, the expressions go to a second rustfmt process that runs alongside the first.
- **Prediction.** The formatter lays the markup out against the input before rustfmt runs, to learn which expressions it will need. A formatted file predicts exactly, which the fixture tests assert; anything a prediction misses goes to rustfmt for every macro in the file at once.
- **Parallel and cached.** Whole-workspace runs format files in parallel, and record each file they leave formatted by a hash of its contents, its settings, the rustfmt configuration, and the rustfmt binary. The next run skips those files without starting rustfmt.

## Benchmarks

On an Apple M2 Max, a 4,178-line file (115 KB) holding 176 `view!` macros, built from [`benches/fixtures/dashboard.rs`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_fmt/benches/fixtures/dashboard.rs), against [leptosfmt](https://github.com/bram209/leptosfmt) 0.1.33 with this repository's former configuration (`hyperfine`, mean of 30 runs):

| Formatting the large file                         |     Time |
| ------------------------------------------------- | -------: |
| nightly rustfmt alone, for reference              |  67.1 ms |
| `leptoskit format`, already formatted             |  83.6 ms |
| `leptosfmt --rustfmt --nightly`, formatted        | 115.2 ms |
| `leptoskit format`, every indentation stripped    | 144.7 ms |
| `leptosfmt --rustfmt --nightly`, same input       | 114.0 ms |
| `leptoskit format` with rustfmt replaced by `cat` |  31.8 ms |
| leptosfmt formatting the markup alone             |  35.1 ms |

The formatter's own work on that file, parsing and laying out all of the markup, takes about 22 ms; the rest is rustfmt. An unformatted file costs more than a formatted one because its macros move, so the expressions inside them go to rustfmt in a second round once their place is known.

This repository's 335 Rust files, with 12 jobs:

| Checking the workspace                                         |    Time |
| -------------------------------------------------------------- | ------: |
| leptosfmt and rustfmt per file, as dprint ran them before      |  3.38 s |
| `leptoskit format --check`, first run                          |  2.38 s |
| `leptoskit format --check`, files unchanged since the last run | 29.7 ms |
