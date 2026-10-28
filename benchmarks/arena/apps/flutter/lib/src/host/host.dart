/// What the app needs from the platform it runs on: its launch arguments, how
/// long its process has been alive, and where report lines go.
///
/// Native platforms use `host_io.dart`; the web uses `host_web.dart`.
library;

export 'host_io.dart' if (dart.library.js_interop) 'host_web.dart';
