---
"release": minor
---

# Draw password fields in generated apps

The macOS, iOS, and Android apps `leptoskit new` generates now draw `PasswordInput`, as an `NSSecureTextField`, a secure `UITextField`, and a password `EditText`. Before, a password field stopped those apps with an "unsupported widget" error.

`autocomplete` hints now reach native password managers too. `username`, `email`, `current-password`, and `new-password` set the field's content type on macOS and iOS, its autofill hint on Android, and its input type on HarmonyOS, in the generated apps and in the AppKit, UIKit, and Android backends. `NativeProps::text_content` reads the hint as a `TextContent`. A `PasswordInput` turns off autocorrection, spell checking, and suggestions on iOS and Android, and an Android password field without a hint is offered to autofill as a password.
