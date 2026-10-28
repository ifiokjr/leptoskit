import 'package:flutter/widgets.dart';

/// A scenario's phase, shown by the beacon so the harness can read it from a
/// screen recording. The colours are part of the arena contract and appear
/// nowhere else on screen.
enum Phase {
  idle(Color(0xFF000000)),
  running(Color(0xFFFF0000)),
  done(Color(0xFF00FF00)),
  failed(Color(0xFF0000FF));

  const Phase(this.color);

  final Color color;
}

/// Screen and text colours from the arena contract.
const Color screenBackground = Color(0xFFFFFFFF);
const Color ink = Color(0xFF111111);

/// Neutral colours for scenario content. None of them is a beacon colour.
const Color hairline = Color(0xFFDDDDDD);
const Color tileFill = Color(0xFFF2F2F2);
