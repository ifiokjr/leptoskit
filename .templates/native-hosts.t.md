<!-- {@generatedHostLimits} -->
Generated macOS, iOS, and Android apps use the Swift or Kotlin renderers in
`leptoskit_bridge_ffi`. They support `View`, `Text`, `Button`, `Pressable`,
`TextInput`, `PasswordInput`, `ScrollView`, `Image`, `Vector`, and `Rive`.
Other control kinds fail with an unsupported-widget error. Accessible names
are forwarded; a labelled `Vector` is an accessible image and `Rive` can be
decorative. Most other roles and states are not forwarded. The Rust backend
crates support a wider control set. Generated Windows and Linux apps use the
full Rust backends, but currently omit runtime plugin registration. Automatic
code push update checks currently run only in generated iOS and Android hosts.
<!-- {/generatedHostLimits} -->
