import 'dart:async';

import 'package:flutter/material.dart';

import '../arena_frame.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';
import '../stats.dart';
import 'feed.dart';

/// `busy`: the feed animates to item 1,500 with `animateTo` while a periodic
/// timer blocks the UI isolate for 40 ms of every 100 ms.
///
/// Flutter has no off-thread scroll animation: `animateTo` ticks on the same
/// isolate the timer blocks, which is exactly what this scenario exposes.
class BusyScenario extends StatefulWidget {
  const BusyScenario({super.key});

  @override
  State<BusyScenario> createState() => _BusyScenarioState();
}

class _BusyScenarioState extends State<BusyScenario> {
  static const int targetItem = 1500;
  static const Duration scrollFor = Duration(milliseconds: 4000);
  static const Duration blockEvery = Duration(milliseconds: 100);
  static const Duration blockFor = Duration(milliseconds: 40);

  final ScrollController _controller = ScrollController();
  Phase _phase = Phase.idle;
  Timer? _blocker;
  int _blocks = 0;

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    await settle();
    if (!mounted) return;

    const target = targetItem * Feed.itemExtent;
    final clock = Stopwatch()..start();
    setState(() => _phase = Phase.running);
    _blocker = Timer.periodic(blockEvery, (_) => _block());
    await _controller.animateTo(
      target,
      duration: scrollFor,
      curve: Curves.linear,
    );
    _blocker?.cancel();
    final scrollMs = clock.elapsedMicroseconds / 1000;

    final shown = _controller.offset;
    final ok = (shown - target).abs() < 0.01;
    setState(() => _phase = ok ? Phase.done : Phase.failed);
    await publishReport(
      ArenaReport(
        scenario: Scenario.busy,
        ok: ok,
        error: ok ? null : 'the feed stopped at $shown, not $target',
        notes: const [
          'animateTo runs on the UI isolate that the 40 ms blocks occupy; '
              'Flutter has no compositor-driven scroll animation',
        ],
        metrics: {'blocks_count': _blocks, 'scroll_ms': round3(scrollMs)},
      ),
    );
  }

  /// Busy-waits, holding the UI isolate as a heavy piece of app logic would.
  void _block() {
    final clock = Stopwatch()..start();
    while (clock.elapsed < blockFor) {}
    _blocks += 1;
  }

  @override
  void dispose() {
    _blocker?.cancel();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.busy.name,
      phase: _phase,
      child: Feed(controller: _controller),
    );
  }
}
