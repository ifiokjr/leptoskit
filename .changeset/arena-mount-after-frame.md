---
"release": patch
---

# Time the arena by each app after-frame hook

`mount` no longer lands in a single frame for every framework, and the beacon recording stopped being able to time it: leptoskit's steps take one or two display frames, so Android's recorder often encodes neither the running nor the done frame, and when it drops the done frame the window spans two steps. The benchmarks chapter's three big-bang rows now read `create_ms`, `update_ms`, and `clear_ms`, which each app reports from its own state change to its earliest after-frame hook, with the hook in the results file. `create_to_pixel_ms`, `update_to_pixel_ms`, and `clear_to_pixel_ms` are still reported wherever the recording has the frames.

Two harness fixes came with it: an idle beacon now closes a window in progress, so a dropped done frame costs one sample instead of inflating the next one, and the recorder keeps running for three seconds after a report instead of one, which was losing the done frame of any scenario that ends on it (`ticker` failed on every leptoskit launch).
