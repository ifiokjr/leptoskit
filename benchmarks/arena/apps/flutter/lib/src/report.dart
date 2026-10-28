import 'dart:convert';

import 'package:flutter/services.dart';

import 'host/host.dart';
import 'scenario.dart';

/// The `ARENA {json}` report a scenario writes when it finishes.
final class ArenaReport {
  const ArenaReport({
    required this.scenario,
    required this.ok,
    this.lane = 'idiomatic',
    this.error,
    this.notes = const [],
    this.metrics = const {},
    this.samples = const {},
  });

  final Scenario scenario;
  final bool ok;
  final String lane;
  final String? error;
  final List<String> notes;
  final Map<String, num> metrics;

  /// At most [maxSamplesPerKey] values per key.
  final Map<String, List<num>> samples;

  static const int maxSamplesPerKey = 100;

  Map<String, Object> toJson() {
    for (final MapEntry(:key, :value) in samples.entries) {
      if (value.length > maxSamplesPerKey) {
        throw ArgumentError('samples[$key] holds ${value.length} values');
      }
    }
    return {
      'v': 1,
      'framework': 'flutter',
      'frameworkVersion': frameworkVersion,
      'platform': arenaPlatform,
      'scenario': scenario.name,
      'lane': lane,
      'ok': ok,
      'error': ?error,
      if (notes.isNotEmpty) 'notes': notes,
      'metrics': metrics,
      if (samples.isNotEmpty) 'samples': samples,
    };
  }
}

/// The Flutter release the app was built with. The `flutter` tool always
/// defines it, so a missing value means the build was not made by the tool.
String get frameworkVersion {
  final version = FlutterVersion.version;
  if (version == null) {
    throw StateError('FLUTTER_VERSION is not defined; build with `flutter`');
  }
  return version;
}

/// The longest JSON text written on one line. System logs truncate long lines
/// (`os_log` at about 1 KB, logcat at about 4 KB), so longer reports are split.
const int maxReportChunk = 800;

/// The log lines for one report: `ARENA {json}` when the JSON fits in
/// [maxReportChunk] bytes, otherwise `ARENA-PART <index>/<count> <chunk>` lines
/// with a 1-based index.
List<String> reportLines(String json) {
  if (json.codeUnits.any((unit) => unit > 0x7f)) {
    throw ArgumentError('report JSON must be ASCII to split by bytes');
  }
  if (json.length <= maxReportChunk) return ['ARENA $json'];

  final count = (json.length / maxReportChunk).ceil();
  return [
    for (var index = 0; index < count; index += 1)
      'ARENA-PART ${index + 1}/$count '
          '${json.substring(index * maxReportChunk, ((index + 1) * maxReportChunk).clamp(0, json.length))}',
  ];
}

/// Writes [report] to the platform's standard log, in order.
Future<void> publishReport(ArenaReport report) async {
  for (final line in reportLines(jsonEncode(report.toJson()))) {
    await writeLogLine(line);
  }
}
