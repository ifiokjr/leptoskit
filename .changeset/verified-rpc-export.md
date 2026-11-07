---
"release": patch
---

# Bind RPC evidence to the verified backend

Export RPC specifications from the checked executable under container confinement and hash the exact exported bytes. Record compatibility as not checked; require an explicit operator declaration for backends without RPC. Keep legacy releases from replacing verified history.

Hash executable bytes directly, preserving differences between non-UTF-8 bytes, and normalize Podman image IDs before recording descriptors.
