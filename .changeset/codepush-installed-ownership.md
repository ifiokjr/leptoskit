---
leptoskit_codepush: patch
leptoskit_bridge_ffi: patch
---

# Prevent overlapping owners from corrupting installed updates

Hold an exclusive file lock for the lifetime of each installed-update store. Overlapping opens, including during iOS controller replacement, return a diagnostic before reading or changing selector state. Dropping or destroying the previous owner releases the store for a retry.
