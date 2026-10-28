---
"release": minor
---

# Fold posture and ArkTS plugins on HarmonyOS

HarmonyOS apps can now select runtime plugins written in ArkTS, and `posture` is the first. A plugin crate declares `harmonyos-type` (and optionally `harmonyos-context`) and ships `harmonyos/<Type>.ts`, a class implementing the host's `NativePlugin` interface. Every prepare regenerates `entry/src/main/ets/leptoskit` with the host's plugin contract, the selected plugins, and a registrant. The Node-API runtime gains `register` and `complete`, and each render batch now carries the guest's plugin requests and cancellations. The page hands those to the plugins and returns each answer on a later UI turn. A call the guest stops awaiting cancels its plugin work and drops the answer.

On HarmonyOS, `posture` reads `@ohos.display`. The fold status gives each hinge's state, including the API 15 statuses for both hinges of a trifold. `getCurrentFoldCreaseRegion` gives where each fold crosses the window, and `foldAngleChange` (API 12) gives each hinge's angle. Flip phones answer `isFoldable()` with false, so the plugin reports them as unknown rather than rigid. This has not yet run on a HarmonyOS emulator or device.

Projects created earlier keep their own `Index.ets` and `index.d.ts`. Copy both from a newly created starter to run ArkTS plugins.
