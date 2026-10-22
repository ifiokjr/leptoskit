# leptoskit_build

Build-script support for one Leptoskit application with platform-specific Rust modules. Add it to the application's build dependencies:

```toml
[build-dependencies]
leptoskit_build = "0.1.0"
```

Call it from `build.rs`:

```rust
fn main() -> Result<(), leptoskit_build::Error> {
	leptoskit_build::platforms()
}
```

Then register a shared module in the application:

```rust
leptoskit::platform_module!(pub mod profile = "routes/profile");
```

The path is relative to the application's `src` directory. `routes/profile.rs` is the shared fallback. Adding `routes/profile.ios.rs` automatically replaces it when building with the `ios` feature. The supported features are `web`, `ios`, `android`, `macos`, `windows`, and `linux`; one application build selects at most one. Builds without a platform feature use shared sources.

The helper watches the source directory recursively, so adding or deleting an override takes effect on the next Cargo build. It communicates the selected device through `LEPTOSKIT_PLATFORM`, keeping iOS and Android distinct even though both application guests compile to Wasm. It does not copy application sources or generate a second module tree.

For nested modules, register the child in both the shared parent and its overrides:

```rust
// src/routes/profile.rs and profile.<platform>.rs
leptoskit::platform_module!(pub mod details = "routes/profile/details");
```

The selector uses Rust's `#[path]` rules. A plain `mod details;` in a selected profile file searches beside that file (`src/routes/details.rs`), rather than inside `src/routes/profile/`. An explicit child registration preserves the intended directory while keeping `self`, `super`, and original compiler source locations.
