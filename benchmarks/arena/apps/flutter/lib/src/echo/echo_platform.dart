/// The `ArenaEcho` tiers for the platform the app runs on: a `MethodChannel`
/// and `dart:ffi` natively, JavaScript interop on the web.
library;

export 'echo_io.dart' if (dart.library.js_interop) 'echo_web.dart';
