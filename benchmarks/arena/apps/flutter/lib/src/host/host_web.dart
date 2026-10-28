import 'dart:js_interop';

@JS('performance.now')
external double _performanceNow();

@JS('console.log')
external void _consoleLog(JSString line);

/// The platform name used in reports.
String get arenaPlatform => 'web';

/// The web passes the scenario as `?scenario=<id>`; it is turned into the same
/// arguments the other platforms receive.
Future<List<String>> launchArguments(List<String> mainArguments) async {
  final scenario = Uri.base.queryParameters['scenario'];
  return scenario == null ? const [] : ['--scenario', scenario];
}

/// Milliseconds since navigation started, the page's equivalent of process
/// start.
Future<double?> processUptimeMs() async => _performanceNow();

/// Writes one line to the browser console.
Future<void> writeLogLine(String line) async => _consoleLog(line.toJS);
