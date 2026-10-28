# opencode, in leptoskit

A coding-agent interface modelled on opencode 2's terminal app, written once as file routes over shared components and built by the leptoskit CLI for the browser and the terminal. Every reply is scripted, so it runs offline.

- **Home** (`/`): the two-tone block logo, a prompt with the agent's color bar, and key hints.
- **Session** (`/session`): a tab, the conversation pinned to its newest message (markdown, a collapsed tool group, a diff that splits side by side on wide screens, shell output), the prompt, and a context sidebar on wide screens.
- **Command palette**: grouped commands with search, over a scrim that dims the screen.

```bash
cd examples/opencode
leptoskit dev                   # the browser, rebuilt on every change
leptoskit dev --platform tui    # this terminal
leptoskit build --platform tui --release   # the runner you ship, in .leptoskit/tui/app/
```

Type a prompt and press `Enter`, open the palette with `Ctrl+P` (arrows, `Enter`, and `Esc` drive it), switch agents with `Shift+Tab`, and quit the terminal app with `Ctrl+C`.

## How it is built

- `src/routes/` holds the screens, and `leptoskit routes` writes `src/routes.rs` from it. `_layout.rs` wraps every screen: it takes opencode's shortcuts with `on_key` before the focused field sees them, and draws the palette.
- `Store` holds the state every route reads; the prompt keeps focus with `focused(true)`, and the palette's search takes it while the palette is open.
- Sizes are whole or half cells: `px-4` is two columns, `py-4` one row, and the prompt's `pb-2` is the half row the terminal closes with `▀`. The sidebar and the split diff appear at breakpoints (`min-[968px]`, `min-[1336px]`), which the browser and the terminal both resolve.
- `web/index.html` sets a monospace font with 8×16-pixel cells, so the browser lays the tree out on the terminal's grid.
- `tui/` is the terminal runner as `leptoskit new` generates it. It is a Cargo workspace of its own, built only by the CLI.
