import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../arena_frame.dart';
import '../inspect.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';

/// `ticker`: a 15 × 20 grid of tiles, every tile rewritten through `setState`
/// on every [Ticker] callback for five seconds.
class TickerScenario extends StatefulWidget {
  const TickerScenario({super.key});

  @override
  State<TickerScenario> createState() => _TickerScenarioState();
}

class _TickerScenarioState extends State<TickerScenario>
    with SingleTickerProviderStateMixin {
  static const int columns = 15;
  static const int rows = 20;
  static const Duration runFor = Duration(milliseconds: 5000);

  late final Ticker _ticker = createTicker(_onTick);
  final GlobalKey _tileZero = GlobalKey();
  Phase _phase = Phase.idle;

  /// The logic updates applied so far; tile `i` shows `frame + i`.
  int _frame = 0;
  int _skipped = 0;
  Duration? _previousTick;
  late Duration _lateAfter;

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    await settle();
    if (!mounted) return;
    final refreshRate = View.of(context).display.refreshRate;
    // A callback more than 1.5 vsyncs after the previous one skipped a frame.
    _lateAfter = Duration(microseconds: (1.5e6 / refreshRate).round());
    _ticker.start();
  }

  void _onTick(Duration elapsed) {
    if (_previousTick case final previous?
        when elapsed - previous > _lateAfter) {
      _skipped += 1;
    }
    _previousTick = elapsed;
    final finished = elapsed >= runFor;
    setState(() {
      _frame += 1;
      _phase = finished ? Phase.done : Phase.running;
    });
    if (finished) {
      _ticker.stop();
      _finish();
    }
  }

  Future<void> _finish() async {
    await SchedulerBinding.instance.endOfFrame;
    final shown = shownText(_tileZero.currentContext! as Element);
    final ok = shown == '$_frame';
    if (!ok) setState(() => _phase = Phase.failed);
    await publishReport(
      ArenaReport(
        scenario: Scenario.ticker,
        ok: ok,
        error: ok ? null : 'tile 0 shows "$shown", not "$_frame"',
        metrics: {
          'frames_count': _frame,
          'skipped_count': _skipped,
          'final_frame': _frame,
        },
      ),
    );
  }

  @override
  void dispose() {
    _ticker.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.ticker.name,
      phase: _phase,
      child: Column(
        children: [
          for (var row = 0; row < rows; row += 1)
            Expanded(
              child: Row(
                children: [
                  for (var column = 0; column < columns; column += 1)
                    Expanded(
                      child: _Tile(
                        textKey: row == 0 && column == 0 ? _tileZero : null,
                        value: _frame + row * columns + column,
                      ),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({required this.textKey, required this.value});

  final Key? textKey;
  final int value;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: tileFill,
        border: Border.all(color: screenBackground, width: 0.5),
      ),
      child: Center(
        child: Text(
          '$value',
          key: textKey,
          maxLines: 1,
          style: const TextStyle(fontSize: 10),
        ),
      ),
    );
  }
}
