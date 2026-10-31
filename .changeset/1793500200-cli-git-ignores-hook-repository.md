---
"release": patch
---

# The CLI reads the project's git repository inside hooks

Git sets `GIT_DIR` and related variables for hooks, so `leptoskit dev` and the `db` commands run from a hook, such as a pre-push hook in a linked worktree, read the hook's repository instead of the project's: the wrong branch, and so the wrong branch database. Every `git` call the CLI makes now finds the repository from the project's directory alone.
