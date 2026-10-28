/// Bindings to `src/arena_echo.c`, which `hook/build.dart` compiles into the
/// app as a code asset with this library's URI as its id.
library;

import 'dart:ffi';

@Native<Void Function()>(symbol: 'arena_echo_none', isLeaf: true)
external void arenaEchoNone();

@Native<Int64 Function(Int64)>(symbol: 'arena_add_one', isLeaf: true)
external int arenaAddOne(int value);

@Native<Void Function(Pointer<Uint8>, Size, Pointer<Uint8>)>(
  symbol: 'arena_echo_bytes',
  isLeaf: true,
)
external void arenaEchoBytes(
  Pointer<Uint8> input,
  int length,
  Pointer<Uint8> output,
);

@Native<Void Function(Pointer<Int64>, Size, Pointer<Int64>)>(
  symbol: 'arena_echo_int64s',
  isLeaf: true,
)
external void arenaEchoInt64s(
  Pointer<Int64> input,
  int count,
  Pointer<Int64> output,
);
