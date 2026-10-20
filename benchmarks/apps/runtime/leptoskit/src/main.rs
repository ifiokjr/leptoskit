//! The server binary `leptoskit build` compiles alongside the wasm client.
//!
//! The benchmark serves the built `dist/` statically, so the server only has
//! to exist; `leptoskit serve` does the real work when benchmarking locally.

fn main() {
	println!("tablebench: serve dist/ with `leptoskit serve`");
}
