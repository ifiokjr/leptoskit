# Agent UI guest probe

A compiled guest that renders an agent's document with `leptoskit_agent_ui` inside the native interpreter, the way a published app runs it. `crates/leptoskit_wasm/tests/agent_ui_guest.rs` loads it, presses the document's stepper and button, and checks that the text reading the variable and the application's action handler both follow. Run it with `devenv shell test:plugins:guest`.
