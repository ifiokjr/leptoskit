---
"release": patch
---

# Job workers wake when the change listener starts

Starting job workers now wakes them once their database change listener is ready. A job enqueued between a worker's first queue scan and listener startup no longer waits for the next polling interval.
