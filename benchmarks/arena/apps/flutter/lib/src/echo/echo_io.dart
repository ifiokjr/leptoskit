import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:flutter/services.dart';

import 'arena_echo_ffi.dart';
import 'echo_module.dart';

AsyncEcho createAsyncEcho() => const _MethodChannelEcho();

SyncEcho createSyncEcho() => const _FfiEcho();

/// `ArenaEcho` behind a `MethodChannel` with the standard codec, answered by
/// Swift on iOS and macOS, Kotlin on Android, and C++ on Windows and Linux.
final class _MethodChannelEcho implements AsyncEcho {
  const _MethodChannelEcho();

  static const MethodChannel _channel = MethodChannel('arena/echo');

  @override
  String get description =>
      'async: MethodChannel (StandardMethodCodec) to a platform-language '
      'handler; bytes travel as Uint8List';

  @override
  Future<void> none() => _channel.invokeMethod<void>('echo');

  @override
  Future<int?> addOne(int value) => _channel.invokeMethod<int>('addOne', value);

  @override
  Future<String?> echoString(String value) =>
      _channel.invokeMethod<String>('echo', value);

  @override
  Future<Uint8List?> echoBytes(Uint8List value) =>
      _channel.invokeMethod<Uint8List>('echo', value);

  @override
  Future<Map<Object?, Object?>?> echoObject(Map<String, int> value) =>
      _channel.invokeMethod<Map<Object?, Object?>>('echo', value);
}

/// `ArenaEcho` as C functions called through `dart:ffi` leaf calls. Typed data
/// is passed by address with no copy into native memory.
final class _FfiEcho implements SyncEcho {
  const _FfiEcho();

  static const int _fieldCount = 100;

  @override
  String get description =>
      'sync: dart:ffi leaf calls into C compiled by a build hook; strings are '
      'UTF-8 encoded into a Uint8List, and the object travels as a '
      '100-element Int64List in field order because C has no maps';

  @override
  void none() => arenaEchoNone();

  @override
  int addOne(int value) => arenaAddOne(value);

  @override
  String echoString(String value) => utf8.decode(echoBytes(utf8.encode(value)));

  @override
  Uint8List echoBytes(Uint8List value) {
    final output = Uint8List(value.length);
    arenaEchoBytes(value.address, value.length, output.address);
    return output;
  }

  @override
  Map<String, int> echoObject(Map<String, int> value) {
    final input = Int64List(_fieldCount);
    for (var field = 0; field < _fieldCount; field += 1) {
      input[field] = value['f$field']!;
    }
    final output = Int64List(_fieldCount);
    arenaEchoInt64s(input.address, _fieldCount, output.address);
    return {
      for (var field = 0; field < _fieldCount; field += 1)
        'f$field': output[field],
    };
  }
}
