# opencode terminal runner

The host that runs the opencode example in a terminal, as `leptoskit new` generates a runner for the `tui` platform. It holds no screens of its own: it starts the terminal, loads the app's wasm guest, and delivers key presses and resizes to it. The screens live in the example's `src/routes/`.

`leptoskit build --platform tui` writes this runner's registrant into `../.leptoskit/tui/` and builds it, so it is a Cargo workspace of its own rather than a member of the leptoskit workspace. Run it through the CLI from `examples/opencode`:

```bash
leptoskit dev --platform tui
leptoskit build --platform tui --release   # the binary you ship, in .leptoskit/tui/app/
```
