# Offline Rive test runtime

`rive.js.gz` and `rive.wasm` contain the unmodified `@rive-app/canvas@2.45.0` artifacts from the official Rive npm package, downloaded from `https://cdn.jsdelivr.net/npm/@rive-app/canvas@2.45.0/`. The JavaScript is compressed with `gzip -n`; decompress it to inspect the upstream source. The upstream MIT license is in `LICENSE.rive`. These are test-only and excluded from the published crate.

SHA-256 of the uncompressed artifacts:

```text
e5447965db93ee3d1d6d853fe88879ccd9e39f277f769095606c591950397841  rive.js
b462c93977ee0b36ace4a03ad48de83b1e62b158943867fbf81d623f146e9683  rive.wasm
```

`scene/` is our original, script-free test animation, authored with Rive CLI 1.7.0. Rebuild with `rive scene --once`, run `rive scene --verify` and `rive inspect scene --summary`, and inspect a screenshot from `rive scene --screenshot --advance=20`. The exported `scene/build/leptoskit-rive-test.riv` is intentionally checked in so ordinary tests require neither the authoring CLI nor network access.
