---
"release": minor
---

# A calls timeline for the devtools

Plugin bridge calls and retrying server calls can now be recorded for leptoskit's devtools.

- **`leptoskit_bridge`'s `devtools` feature.** Every call's timeline: when the guest submitted it, when the host picked it up, when it answered or was cancelled, when the guest read the answer, and when a host answered a call that was already cancelled, with the size of its arguments and answer. `leptoskit_bridge::devtools::drain()` takes the records. Recording does not allocate per call.
- **`leptoskit_rpc`'s `devtools` feature.** Every attempt `call_with_retry`, `query`, and `mutation_with_retry` make, with the class of error it failed with, through `leptoskit_rpc::devtools::drain()`. `FnError::class()` names that class.
- **Bounded.** Both keep the latest 256 records and count what they drop. Without the feature, nothing is compiled in.
- **`leptoskit_devtools_protocol`.** `CallRecord` gains optional `running_ms`, `taken_ms`, `completed_late_ms`, `request_bytes`, and `response_bytes`, and `sink::Sink` is the bounded buffer both crates record into. The protocol version is unchanged.
