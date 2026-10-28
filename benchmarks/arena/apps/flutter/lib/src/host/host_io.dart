import 'dart:io';

import 'package:flutter/services.dart';

/// The Runner's side of the app on iOS, macOS, and Android. Windows and Linux
/// pass their command line to `main` and need no channel.
const MethodChannel _host = MethodChannel('arena/host');

bool get _hasHostChannel =>
    Platform.isIOS || Platform.isMacOS || Platform.isAndroid;

/// The platform name used in reports: `ios`, `android`, `macos`, `windows`,
/// or `linux`.
String get arenaPlatform => Platform.operatingSystem;

/// The launch arguments. iOS and macOS hand them over from the Runner, and
/// Android turns its `scenario` intent extra into `--scenario <id>`.
Future<List<String>> launchArguments(List<String> mainArguments) async {
  if (!_hasHostChannel) return mainArguments;
  final arguments = await _host.invokeListMethod<String>('arguments');
  return arguments ?? const [];
}

/// Milliseconds since the process started, or null where the platform does not
/// expose a process start time cheaply (Windows).
Future<double?> processUptimeMs() async {
  if (_hasHostChannel) return _host.invokeMethod<double>('uptimeMs');
  if (Platform.isLinux) return _linuxProcessUptimeMs();
  return null;
}

/// Writes one line to the platform's standard log: logcat with the tag `ARENA`
/// on Android, `NSLog` on iOS and macOS, and standard output elsewhere.
Future<void> writeLogLine(String line) async {
  if (_hasHostChannel) {
    await _host.invokeMethod<void>('log', line);
    return;
  }
  stdout.writeln(line);
  await stdout.flush();
}

/// Reads the process start time from `/proc`. `starttime` (field 22 of
/// `/proc/self/stat`) counts clock ticks since boot, and `USER_HZ` is 100 on
/// every Linux ABI, so this has 10 ms resolution.
double? _linuxProcessUptimeMs() {
  final stat = File('/proc/self/stat').readAsStringSync();
  // The command name (field 2) may contain spaces, so count from its `)`.
  final fields = stat.substring(stat.lastIndexOf(')') + 2).split(' ');
  final startTicks = int.parse(fields[19]);
  final uptime = File('/proc/uptime').readAsStringSync().split(' ').first;
  return double.parse(uptime) * 1000 - startTicks * 10;
}
