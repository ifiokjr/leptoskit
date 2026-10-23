---
"release": minor
---

# Identify interpreter and transport owners

Bridge and WasmHost expose opaque cloneable identity handles that remain stable across moves and cannot collide while an earlier handle remains alive. Services can bind work to both owners and reject delivery after replacement or transport transplantation. Handles retain no guest or interpreter state.

App::with_host lends the application's actual interpreter to a synchronous Rust closure. Native I/O can retain owned work while the application remains usable; interpreter references and futures borrowing them cannot escape the closure.
