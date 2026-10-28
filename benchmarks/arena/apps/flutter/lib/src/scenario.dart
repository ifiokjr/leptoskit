import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

/// The arena's scenarios, in the order the home screen lists them.
enum Scenario {
  cold('Cold open'),
  mount('Big bang'),
  echo('Echo ladder'),
  ticker('Ticker wall'),
  autoscroll('Autoscroll'),
  busy('Busy hands');

  const Scenario(this.title);

  final String title;

  /// Reads `--scenario <id>` or `--scenario=<id>`. Returns null when no
  /// scenario was passed, and throws for an unknown id so a typo never runs
  /// the home screen silently.
  static Scenario? fromArguments(List<String> arguments) {
    String? id;
    for (var index = 0; index < arguments.length; index += 1) {
      final argument = arguments[index];
      if (argument == '--scenario' && index + 1 < arguments.length) {
        id = arguments[index + 1];
      } else if (argument.startsWith('--scenario=')) {
        id = argument.substring('--scenario='.length);
      }
    }
    if (id == null || id.isEmpty) return null;
    for (final scenario in values) {
      if (scenario.name == id) return scenario;
    }
    throw ArgumentError.value(id, 'scenario', 'unknown arena scenario');
  }
}

/// How long a scenario waits after its first frame reaches the screen before
/// it starts, so launch work (asset loading, shader warm-up) settles and the
/// beacon shows a clear idle phase.
const Duration settleDelay = Duration(milliseconds: 500);

/// Completes once the app's first frame is on screen.
///
/// Native engines report the first frame's raster timing as soon as it is
/// drawn. The web engine renders inside the frame itself but reports timings
/// in batches flushed by later frames, so there the end of the first frame is
/// the moment it reached the canvas.
Future<void> firstFrameShown() => kIsWeb
    ? SchedulerBinding.instance.endOfFrame
    : WidgetsBinding.instance.waitUntilFirstFrameRasterized;

/// Waits for the first frame to reach the screen, then for [settleDelay].
Future<void> settle() async {
  await firstFrameShown();
  await Future<void>.delayed(settleDelay);
}
