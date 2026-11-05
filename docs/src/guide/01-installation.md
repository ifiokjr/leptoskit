# Installation

The `leptoskit` command line tool is published on crates.io as `leptoskit_cli`. Install it with cargo:

```bash
cargo install leptoskit_cli --locked
```

The install places two binaries: `leptoskit`, and `lk` — a shorter alias for the same CLI. Everything in this guide works identically through the alias: `lk new my_app`, `lk dev`, `lk build`. Help and error messages answer to whichever name you type.

Web builds compile your client crate to wasm, so add the target to your toolchain:

```bash
rustup target add wasm32-unknown-unknown
```

Release builds run [`wasm-opt`](https://github.com/WebAssembly/binaryen) when it is on your `PATH`. It ships with binaryen, which most package managers carry:

```bash
brew install binaryen      # macOS
sudo apt install binaryen  # Debian and Ubuntu
```

## Check your setup

`leptoskit doctor` checks the tools a build needs and exits with an error if something required is missing:

```bash
leptoskit doctor
```

Inside a project it checks the tools for each native platform the app ships — Xcode and its simulator runtimes, Swift, Java and the Android SDK, GTK 4 on Linux, CocoaPods while a plugin needs it, and the Rust target each platform compiles for — reporting only the platforms you build. `leptoskit doctor --all` checks every platform this machine could build. It also checks that the project's `wasm-bindgen` version matches the one built into the CLI; a mismatch there is the most common cause of confusing bindgen failures.

## Shell completions

`leptoskit completions` prints a completion script for `bash`, `elvish`, `fish`, `powershell`, or `zsh`:

```bash
leptoskit completions zsh > ~/.zfunc/_leptoskit
```
