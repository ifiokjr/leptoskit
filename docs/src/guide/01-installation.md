# Installation

The `leptoskit` command line tool is published on crates.io as `leptoskit_cli`. Install it with cargo:

```bash
cargo install leptoskit_cli --locked
```

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

Run it inside a project and it also checks that the project's `wasm-bindgen` version matches the one built into the CLI. A mismatch there is the most common cause of confusing bindgen failures.

## Shell completions

`leptoskit completions` prints a completion script for `bash`, `elvish`, `fish`, `powershell`, or `zsh`:

```bash
leptoskit completions zsh > ~/.zfunc/_leptoskit
```
