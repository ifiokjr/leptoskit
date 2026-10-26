# harbor

Harbour, a ferry departures board, and the worked example for `leptoskit`'s `query`, `use`, and `rpc` features together.

- **The RPC** (`src/api.rs`): three leptos server functions — the day's sailings, booking a seat, and the harbour master's log — published with `#[api]`. `leptoskit rpc generate` runs the app, reads the spec, and writes the TypeScript client in `clients/typescript`, which the tests keep in step with the code.
- **The queries** (`src/board.rs`): a cached query per day that keeps the previous day's board on screen while the next loads; a booking mutation that drops the seat count at once, puts it back when the server refuses, and reads the day again either way; and the log as an infinite query, a page at a time.
- **The hooks**: a day picker that cycles through the week, a search that filters only once typing pauses, a harbour clock, a board that reads the day again every half minute, a toggle for departed sailings, and a pinned route kept in storage.

```bash
cargo run -p harbor_example            # a scripted morning at the harbour, printed as a screen reader reads it
cargo test -p harbor_example
```

The example builds in server mode, so the server functions run in-process, the way the server calls them for a request; each answer takes 50 ms, as a network round trip would, so a booking shows on the board before the harbour confirms it. A browser build of the same functions calls them over HTTP on the routes the spec lists.

After changing a server function or a type it returns, run `leptoskit rpc generate` here to rewrite the client; the tests fail while it is out of date.
