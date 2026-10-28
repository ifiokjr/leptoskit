# opencode, in leptoskit

A coding-agent interface modelled on opencode 2's terminal app, written once as a leptoskit element tree and rendered both in an interactive terminal and in the browser. Every reply is scripted, so it runs offline.

- **Home:** the two-tone block logo, a prompt with the agent's color bar, and key hints.
- **Session:** a tab, the conversation pinned to its newest message (markdown, a collapsed tool group, a diff that splits side by side on wide screens, shell output), the prompt, and a context sidebar on wide screens.
- **Command palette:** grouped commands with search, over a scrim that dims the screen.

```bash
cargo run -p opencode_example --bin opencode   # in this terminal
cd examples/opencode && trunk serve            # in the browser
```

Type a prompt and press `Enter`, open the palette with `Ctrl+P` (arrows, `Enter`, and `Esc` drive it), switch agents with `Shift+Tab`, and quit with `Ctrl+C`.

## How it is built

- `src/ui` describes the screens with `View`, `Text`, and `TextInput` and Tailwind classes. Sizes are whole or half cells: `px-4` is two columns, `py-4` one row, and the prompt's `pb-2` is the half row the terminal closes with `▀`.
- Colored text (the logo, syntax highlighting, key hints) uses styled runs: `Text("").spans([...])`.
- `src/terminal.rs` is the terminal host: it owns the state, maps keys to actions through `TuiApplication::shortcut`, and keeps focus on the prompt with `TuiBackend::focus_id`.
- `src/bin/web.rs` mounts the same tree with Leptos; `index.html` sets a monospace font with 8×16-pixel cells so the browser's layout lands on the terminal's grid.
