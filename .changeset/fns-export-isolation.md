---
"release": patch
---

# Isolate concurrent function specification exports

Concurrent CLI exports sharing a Cargo target now use separate temporary output directories. A server that writes no specification reliably reports the missing export, and timeout cleanup joins the server before releasing its output directory.
