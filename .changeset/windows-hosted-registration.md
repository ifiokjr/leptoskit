---
"release": patch
---

# Register hosted projects on Windows

Resolve the broker's placeholder project root to an absolute path on the current platform. Hosted registration now satisfies the journal's absolute-path requirement on Windows as well as Unix, so worker pairing, state storage, and backup restore can proceed.
