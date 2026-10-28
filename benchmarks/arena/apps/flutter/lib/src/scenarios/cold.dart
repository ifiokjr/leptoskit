import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../arena_frame.dart';
import '../host/host.dart';
import '../inspect.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';
import '../stats.dart';

/// `cold`: the ready screen, drawn as done in its very first frame. The
/// harness measures launch to presented frame from the recording; the app adds
/// `first_frame_ms` where the platform exposes its process start time.
class ColdScenario extends StatefulWidget {
  const ColdScenario({super.key, required this.fromLaunch});

  /// Whether this is the app's first screen. Opened from the home screen, the
  /// time since process start means nothing, so it is not reported.
  final bool fromLaunch;

  @override
  State<ColdScenario> createState() => _ColdScenarioState();
}

class _ColdScenarioState extends State<ColdScenario> {
  static const int rowCount = 20;

  final GlobalKey _rows = GlobalKey();

  /// Done from the first frame; failed only if the check fails.
  Phase _phase = Phase.done;

  @override
  void initState() {
    super.initState();
    _report();
  }

  Future<void> _report() async {
    double? firstFrameMs;
    if (widget.fromLaunch) {
      await firstFrameShown();
      final sinceFirstFrame = Stopwatch()..start();
      final uptimeMs = await processUptimeMs();
      if (uptimeMs != null) {
        firstFrameMs = uptimeMs - sinceFirstFrame.elapsedMicroseconds / 1000;
      }
    } else {
      await WidgetsBinding.instance.endOfFrame;
    }
    if (!mounted) return;

    final expected = [for (var row = 1; row <= rowCount; row += 1) 'Row $row'];
    final shown = shownTexts(_rows.currentContext! as Element);
    final ok = listEquals(shown, expected);
    if (!ok) setState(() => _phase = Phase.failed);
    await publishReport(
      ArenaReport(
        scenario: Scenario.cold,
        ok: ok,
        error: ok ? null : 'expected $expected, found $shown',
        notes: [
          if (firstFrameMs != null)
            'first_frame_ms runs from process start to the moment Dart is '
                'told the first frame is on screen (native: '
                'WidgetsBinding.waitUntilFirstFrameRasterized; web: the end of '
                'the first frame, which draws to the canvas synchronously)',
        ],
        metrics: {
          if (firstFrameMs != null) 'first_frame_ms': round3(firstFrameMs),
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.cold.name,
      phase: _phase,
      child: Column(
        key: _rows,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (var row = 1; row <= rowCount; row += 1)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 2),
              child: Text('Row $row'),
            ),
        ],
      ),
    );
  }
}
