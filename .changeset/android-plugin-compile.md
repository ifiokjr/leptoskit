---
"release": patch
---

# Restore Android plugin compilation

Restore the secure-store archive writer removed during the host refactor, preserving authenticated encryption and atomic writes. Add the missing URI import in web auth so Android applications using these plugins compile again.
