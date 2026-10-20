---
"release": minor
---

# Ship `lk` as the CLI's alias binary

`cargo install leptoskit_cli` now installs `lk` next to `leptoskit`: the same entry point built under a shorter name, so `lk new`, `lk dev`, and `lk build` all work. Help and error messages answer to the alias, completion scripts complete the invoked name (`lk completions zsh > ~/.zfunc/_lk`), and the scaffold's next-step hint echoes it.
