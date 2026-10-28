import 'dart:typed_data';

/// The idiomatic, asynchronous way of calling the `ArenaEcho` native module.
abstract interface class AsyncEcho {
  /// What this tier is, recorded in the report's notes.
  String get description;

  Future<void> none();
  Future<int?> addOne(int value);
  Future<String?> echoString(String value);
  Future<Uint8List?> echoBytes(Uint8List value);
  Future<Map<Object?, Object?>?> echoObject(Map<String, int> value);
}

/// The tuned, synchronous way of calling the same module.
abstract interface class SyncEcho {
  /// What this tier is, recorded in the report's notes.
  String get description;

  void none();
  int addOne(int value);
  String echoString(String value);
  Uint8List echoBytes(Uint8List value);
  Map<String, int> echoObject(Map<String, int> value);
}
