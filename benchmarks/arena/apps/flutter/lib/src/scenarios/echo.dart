import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../arena_frame.dart';
import '../echo/echo_module.dart';
import '../echo/echo_platform.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';
import '../stats.dart';

/// `echo`: round trips to the `ArenaEcho` native module with five payloads,
/// through the async tier (idiomatic) and the sync tier (tuned).
class EchoScenario extends StatefulWidget {
  const EchoScenario({super.key});

  @override
  State<EchoScenario> createState() => _EchoScenarioState();
}

/// One payload's call through one tier, and how to check what came back.
typedef _Case<R> = ({
  String payload,
  R Function() invoke,
  bool Function(Object?) verify,
});

class _EchoScenarioState extends State<EchoScenario> {
  static const int warmUpCalls = 200;
  static const int timedCalls = 2000;
  static const int sampleBatch = 100;

  Phase _phase = Phase.idle;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    await settle();
    if (!mounted) return;
    setState(() => _phase = Phase.running);
    // Let the running frame reach the screen before the loops occupy the isolate.
    await WidgetsBinding.instance.endOfFrame;

    final asyncEcho = createAsyncEcho();
    final syncEcho = createSyncEcho();
    final metrics = <String, num>{};
    final samples = <String, List<num>>{};
    try {
      for (final echoCase in _asyncCases(asyncEcho)) {
        _record(
          'async',
          echoCase.payload,
          await _measureAsync(echoCase),
          metrics,
          samples,
        );
      }
      for (final echoCase in _syncCases(syncEcho)) {
        _record(
          'sync',
          echoCase.payload,
          _measureSync(echoCase),
          metrics,
          samples,
        );
      }
    } on _EchoFailed catch (failure) {
      return _fail(failure.reason);
    } on MissingPluginException catch (error) {
      return _fail('ArenaEcho is not registered: ${error.message}');
    } on PlatformException catch (error) {
      return _fail('ArenaEcho failed: ${error.code} ${error.message}');
    }

    setState(() => _phase = Phase.done);
    await publishReport(
      ArenaReport(
        scenario: Scenario.echo,
        ok: true,
        notes: [
          asyncEcho.description,
          '${syncEcho.description}; the sync tier is the tuned lane',
          'samples hold the mean of each batch of $sampleBatch calls under '
              'the p50 key',
        ],
        metrics: metrics,
        samples: samples,
      ),
    );
  }

  Future<void> _fail(String reason) async {
    setState(() => _phase = Phase.failed);
    await publishReport(
      ArenaReport(scenario: Scenario.echo, ok: false, error: reason),
    );
  }

  Future<List<double>> _measureAsync(_Case<Future<Object?>> echoCase) async {
    for (var call = 0; call < warmUpCalls; call += 1) {
      _verify(echoCase, await echoCase.invoke());
    }
    final clock = Stopwatch()..start();
    final latenciesUs = List<double>.filled(timedCalls, 0);
    for (var call = 0; call < timedCalls; call += 1) {
      final start = clock.elapsedTicks;
      final result = await echoCase.invoke();
      latenciesUs[call] = (clock.elapsedTicks - start) * 1e6 / clock.frequency;
      _verify(echoCase, result);
    }
    return latenciesUs;
  }

  List<double> _measureSync(_Case<Object?> echoCase) {
    for (var call = 0; call < warmUpCalls; call += 1) {
      _verify(echoCase, echoCase.invoke());
    }
    final clock = Stopwatch()..start();
    final latenciesUs = List<double>.filled(timedCalls, 0);
    for (var call = 0; call < timedCalls; call += 1) {
      final start = clock.elapsedTicks;
      final result = echoCase.invoke();
      latenciesUs[call] = (clock.elapsedTicks - start) * 1e6 / clock.frequency;
      _verify(echoCase, result);
    }
    return latenciesUs;
  }

  void _verify<R>(_Case<R> echoCase, Object? result) {
    if (!echoCase.verify(result)) {
      throw _EchoFailed('${echoCase.payload} echoed a wrong result');
    }
  }

  /// Adds `<tier>_<payload>_p50_us`, `_p99_us`, and `_per_s` from all calls,
  /// and the batch means as samples.
  void _record(
    String tier,
    String payload,
    List<double> latenciesUs,
    Map<String, num> metrics,
    Map<String, List<num>> samples,
  ) {
    final key = '${tier}_$payload';
    final totalUs = latenciesUs.reduce((a, b) => a + b);
    metrics['${key}_p50_us'] = round3(percentile(latenciesUs, 50));
    metrics['${key}_p99_us'] = round3(percentile(latenciesUs, 99));
    metrics['${key}_per_s'] = round3(latenciesUs.length / totalUs * 1e6);
    samples['${key}_p50_us'] = batchMeans(
      latenciesUs,
      sampleBatch,
    ).map(round3).toList();
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.echo.name,
      phase: _phase,
      child: const Padding(
        padding: EdgeInsets.all(16),
        child: Text(
          'Round trips to ArenaEcho: none, int, str1k, bytes64k, obj',
        ),
      ),
    );
  }
}

final String _str1k = String.fromCharCodes([
  for (var i = 0; i < 1024; i += 1) 0x61 + i % 26,
]);

final Uint8List _bytes64k = Uint8List.fromList([
  for (var i = 0; i < 65536; i += 1) i % 251,
]);

final Map<String, int> _object = {for (var i = 0; i < 100; i += 1) 'f$i': i};

List<_Case<Future<Object?>>> _asyncCases(AsyncEcho echo) => [
  (payload: 'none', invoke: echo.none, verify: (result) => result == null),
  (
    payload: 'int',
    invoke: () => echo.addOne(41),
    verify: (result) => result == 42,
  ),
  (
    payload: 'str1k',
    invoke: () => echo.echoString(_str1k),
    verify: (result) => result == _str1k,
  ),
  (
    payload: 'bytes64k',
    invoke: () => echo.echoBytes(_bytes64k),
    verify: _isBytes64k,
  ),
  (payload: 'obj', invoke: () => echo.echoObject(_object), verify: _isObject),
];

List<_Case<Object?>> _syncCases(SyncEcho echo) => [
  (
    payload: 'none',
    invoke: () {
      echo.none();
      return null;
    },
    verify: (result) => result == null,
  ),
  (
    payload: 'int',
    invoke: () => echo.addOne(41),
    verify: (result) => result == 42,
  ),
  (
    payload: 'str1k',
    invoke: () => echo.echoString(_str1k),
    verify: (result) => result == _str1k,
  ),
  (
    payload: 'bytes64k',
    invoke: () => echo.echoBytes(_bytes64k),
    verify: _isBytes64k,
  ),
  (payload: 'obj', invoke: () => echo.echoObject(_object), verify: _isObject),
];

bool _isBytes64k(Object? result) {
  if (result is! Uint8List || result.length != _bytes64k.length) return false;
  for (var i = 0; i < result.length; i += 1) {
    if (result[i] != i % 251) return false;
  }
  return true;
}

bool _isObject(Object? result) {
  if (result is! Map || result.length != _object.length) return false;
  for (var i = 0; i < _object.length; i += 1) {
    if (result['f$i'] != i) return false;
  }
  return true;
}

final class _EchoFailed implements Exception {
  const _EchoFailed(this.reason);

  final String reason;
}
