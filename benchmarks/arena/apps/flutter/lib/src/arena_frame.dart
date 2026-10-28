import 'package:flutter/material.dart';

import 'phase.dart';

/// The screen every arena scenario draws: a column filling the safe area whose
/// first child is the 48 pt header row (the beacon and the status text).
class ArenaFrame extends StatelessWidget {
  const ArenaFrame({
    super.key,
    required this.label,
    required this.phase,
    required this.child,
  });

  /// The scenario id, or `home`, shown before the phase in the status text.
  final String label;
  final Phase phase;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return ColoredBox(
      color: screenBackground,
      child: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(
              height: 48,
              child: Row(
                children: [
                  Padding(
                    padding: const EdgeInsets.all(8),
                    child: SizedBox.square(
                      dimension: 32,
                      child: ColoredBox(color: phase.color),
                    ),
                  ),
                  Expanded(
                    child: Text(
                      '$label ${phase.name}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 16),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(child: child),
          ],
        ),
      ),
    );
  }
}
