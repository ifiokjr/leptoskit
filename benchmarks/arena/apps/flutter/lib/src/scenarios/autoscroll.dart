import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../arena_frame.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';
import '../stats.dart';
import 'feed.dart';

/// `autoscroll`: the feed scrolled by app logic, one `jumpTo` per [Ticker]
/// callback, at 1.5 pt per millisecond for five seconds.
class AutoscrollScenario extends StatefulWidget {
  const AutoscrollScenario({super.key});

  @override
  State<AutoscrollScenario> createState() => _AutoscrollScenarioState();
}

class _AutoscrollScenarioState extends State<AutoscrollScenario>
    with SingleTickerProviderStateMixin {
  static const double pointsPerMs = 1.5;
  static const Duration runFor = Duration(milliseconds: 5000);

  late final Ticker _ticker = createTicker(_onTick);
  final ScrollController _controller = ScrollController();
  Phase _phase = Phase.idle;
  int _frames = 0;
  double _offset = 0;
  Duration _previousTick = Duration.zero;

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    await settle();
    if (mounted) _ticker.start();
  }

  void _onTick(Duration elapsed) {
    final dtMs = (elapsed - _previousTick).inMicroseconds / 1000;
    _previousTick = elapsed;
    _offset += pointsPerMs * dtMs;
    _controller.jumpTo(_offset);
    _frames += 1;

    final finished = elapsed >= runFor;
    final phase = finished ? Phase.done : Phase.running;
    if (phase != _phase) setState(() => _phase = phase);
    if (finished) {
      _ticker.stop();
      _finish();
    }
  }

  Future<void> _finish() async {
    await SchedulerBinding.instance.endOfFrame;
    final shown = _controller.offset;
    final ok = (shown - _offset).abs() < 0.01;
    if (!ok) setState(() => _phase = Phase.failed);
    await publishReport(
      ArenaReport(
        scenario: Scenario.autoscroll,
        ok: ok,
        error: ok ? null : 'the feed is at $shown, not $_offset',
        metrics: {'frames_count': _frames, 'final_offset': round3(shown)},
      ),
    );
  }

  @override
  void dispose() {
    _ticker.dispose();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.autoscroll.name,
      phase: _phase,
      child: Feed(controller: _controller),
    );
  }
}
