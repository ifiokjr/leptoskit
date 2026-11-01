---
"release": patch
---

# Hold the tree to monostyle's 80 floor

Record the project's monostyle thresholds and run the whole tree from `lint:all` and the pre-push hook, failing below 80 readability or complexity. `devenv.nix` stays at 100.
