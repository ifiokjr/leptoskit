---
release: minor
---

# Retain application services through asynchronous shutdown

`App` can defer one executor-neutral service until native wake and close owners exist. Services borrow the actual interpreter briefly, preserve unrelated requests and cancellations, and receive coalesced notifications from arbitrary threads.

Rust and C close handles stop admission and delivery before teardown, retain the interpreter until owned work has joined, and return cancelled close awaits to the configured owner-thread scheduler. Embedders must retain and poll closing owners until completion. Generated Swift, Kotlin, JNI and CLI service wiring remains separate work.
