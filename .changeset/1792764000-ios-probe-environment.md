---
"release": patch
---

# Preserve the selected Rust installation in the iOS probe

The simulator probe keeps the caller-selected Cargo, rustc and toolchain when clearing Nix build variables. Missing tools or an uninstalled simulator target now fail visibly before simulator work, and a maintained boundary control covers Rust installations outside the default Cargo home.
