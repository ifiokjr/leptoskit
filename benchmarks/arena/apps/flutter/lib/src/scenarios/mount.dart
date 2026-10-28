import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import '../arena_frame.dart';
import '../inspect.dart';
import '../phase.dart';
import '../report.dart';
import '../scenario.dart';
import '../stats.dart';

/// `mount`: five rounds of creating 1,000 cards, retitling every tenth, and
/// clearing them, in a non-virtualised scroll view.
///
/// Each step is timed from `setState` to [SchedulerBinding.endOfFrame], the
/// earliest after-frame hook: it completes once the frame is built, laid out,
/// and painted, before the raster thread draws it.
class MountScenario extends StatefulWidget {
  const MountScenario({super.key});

  @override
  State<MountScenario> createState() => _MountScenarioState();
}

class _MountScenarioState extends State<MountScenario> {
  static const int rounds = 5;
  static const int cardCount = 1000;

  final GlobalKey _cards = GlobalKey();
  Phase _phase = Phase.idle;
  List<String> _titles = const [];

  @override
  void initState() {
    super.initState();
    _run();
  }

  Future<void> _run() async {
    await settle();
    final create = <double>[];
    final update = <double>[];
    final clear = <double>[];
    try {
      for (var round = 0; round < rounds; round += 1) {
        create.add(
          await _step(
            () => _titles = [for (var i = 0; i < cardCount; i += 1) 'Card $i'],
            expected: (count: cardCount, first: 'Card 0', last: 'Card 999'),
          ),
        );
        update.add(
          await _step(
            () => _titles = [
              for (final (i, title) in _titles.indexed)
                i % 10 == 0 ? '$title !' : title,
            ],
            expected: (count: cardCount, first: 'Card 0 !', last: 'Card 999'),
          ),
        );
        clear.add(
          await _step(
            () => _titles = const [],
            expected: (count: 0, first: null, last: null),
          ),
        );
      }
    } on _CheckFailed catch (failure) {
      setState(() => _phase = Phase.failed);
      await publishReport(
        ArenaReport(scenario: Scenario.mount, ok: false, error: failure.reason),
      );
      return;
    }

    setState(() => _phase = Phase.done);
    await publishReport(
      ArenaReport(
        scenario: Scenario.mount,
        ok: true,
        notes: const [
          'each step is timed from setState to SchedulerBinding.endOfFrame '
              '(built, laid out, and painted; before rasterization)',
        ],
        metrics: {
          'create_ms': round3(median(create)),
          'update_ms': round3(median(update)),
          'clear_ms': round3(median(clear)),
        },
        samples: {
          'create_ms': create.map(round3).toList(),
          'update_ms': update.map(round3).toList(),
          'clear_ms': clear.map(round3).toList(),
        },
      ),
    );
  }

  /// Applies [change] with the beacon running, times it to the end of its
  /// frame, checks what was rendered, then shows done and an idle frame.
  Future<double> _step(
    void Function() change, {
    required ({int count, String? first, String? last}) expected,
  }) async {
    final clock = Stopwatch()..start();
    setState(() {
      change();
      _phase = Phase.running;
    });
    await SchedulerBinding.instance.endOfFrame;
    final elapsedMs = clock.elapsedMicroseconds / 1000;

    _check(expected);
    setState(() => _phase = Phase.done);
    await SchedulerBinding.instance.endOfFrame;
    setState(() => _phase = Phase.idle);
    await SchedulerBinding.instance.endOfFrame;
    return elapsedMs;
  }

  void _check(({int count, String? first, String? last}) expected) {
    final cards = descendantsOfType<_Card>(_cards.currentContext! as Element);
    if (cards.length != expected.count) {
      throw _CheckFailed(
        'rendered ${cards.length} cards, not ${expected.count}',
      );
    }
    if (cards.isEmpty) return;
    final first = shownTexts(cards.first).first;
    final last = shownTexts(cards.last).first;
    if (first != expected.first || last != expected.last) {
      throw _CheckFailed(
        'first and last cards read "$first" and "$last", '
        'not "${expected.first}" and "${expected.last}"',
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: Scenario.mount.name,
      phase: _phase,
      child: SingleChildScrollView(
        child: Column(
          key: _cards,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final (index, title) in _titles.indexed)
              _Card(key: ValueKey(index), index: index, title: title),
          ],
        ),
      ),
    );
  }
}

class _Card extends StatelessWidget {
  const _Card({super.key, required this.index, required this.title});

  final int index;
  final String title;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: const BoxDecoration(
        border: Border(bottom: BorderSide(color: hairline)),
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title, style: const TextStyle(fontSize: 16)),
            Text('Subtitle $index', style: const TextStyle(fontSize: 13)),
          ],
        ),
      ),
    );
  }
}

final class _CheckFailed implements Exception {
  const _CheckFailed(this.reason);

  final String reason;
}
