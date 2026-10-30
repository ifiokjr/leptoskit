# leptoskit_rive_runtime

The pinned, offline C++ dependency used by `leptoskit_rive` native playback. Applications use `leptoskit_rive`; this crate exposes no application API. Its separate Cargo build lets changes to the Rust player or image adapter reuse the C++ compilation cache.

Native builds require a C++17 compiler and the target platform's SDK. Browser Wasm and WASI never compile the native sources. Source provenance and its MIT license are included in `vendor/`.

The private C++ library uses optimization level 1 in development and 2 in release, without C++ debug symbols. Rust debug information and upstream runtime assertions retain their normal profile behavior. Supply compiler debug flags explicitly when investigating upstream C++ code.
