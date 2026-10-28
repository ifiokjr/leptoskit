import 'package:flutter/material.dart';

import 'src/arena_frame.dart';
import 'src/host/host.dart';
import 'src/phase.dart';
import 'src/scenario.dart';
import 'src/scenarios/autoscroll.dart';
import 'src/scenarios/busy.dart';
import 'src/scenarios/cold.dart';
import 'src/scenarios/echo.dart';
import 'src/scenarios/mount.dart';
import 'src/scenarios/ticker.dart';

/// The Flutter arena app. Launched with a scenario it runs it and reports;
/// launched without one it shows a home screen to run any of them by hand.
Future<void> main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  final scenario = Scenario.fromArguments(await launchArguments(arguments));
  runApp(ArenaApp(launchScenario: scenario));
}

class ArenaApp extends StatefulWidget {
  const ArenaApp({super.key, required this.launchScenario});

  final Scenario? launchScenario;

  @override
  State<ArenaApp> createState() => _ArenaAppState();
}

class _ArenaAppState extends State<ArenaApp> {
  late Scenario? _scenario = widget.launchScenario;

  @override
  Widget build(BuildContext context) {
    final theme = ThemeData(
      colorSchemeSeed: const Color(0xFF3D5A80),
      scaffoldBackgroundColor: screenBackground,
    );
    return MaterialApp(
      title: 'Arena',
      debugShowCheckedModeBanner: false,
      theme: theme.copyWith(
        textTheme: theme.textTheme.apply(bodyColor: ink, displayColor: ink),
      ),
      home: Material(
        color: screenBackground,
        child: switch (_scenario) {
          null => _Home(
            onRun: (scenario) => setState(() => _scenario = scenario),
          ),
          final scenario => _scenarioScreen(
            scenario,
            fromLaunch: scenario == widget.launchScenario,
          ),
        },
      ),
    );
  }
}

Widget _scenarioScreen(Scenario scenario, {required bool fromLaunch}) {
  return switch (scenario) {
    Scenario.cold => ColdScenario(fromLaunch: fromLaunch),
    Scenario.mount => const MountScenario(),
    Scenario.echo => const EchoScenario(),
    Scenario.ticker => const TickerScenario(),
    Scenario.autoscroll => const AutoscrollScenario(),
    Scenario.busy => const BusyScenario(),
  };
}

/// One button per scenario, for running them by hand.
class _Home extends StatelessWidget {
  const _Home({required this.onRun});

  final ValueChanged<Scenario> onRun;

  @override
  Widget build(BuildContext context) {
    return ArenaFrame(
      label: 'home',
      phase: Phase.idle,
      child: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          for (final scenario in Scenario.values)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: FilledButton(
                onPressed: () => onRun(scenario),
                child: Text('${scenario.title} (${scenario.name})'),
              ),
            ),
        ],
      ),
    );
  }
}
