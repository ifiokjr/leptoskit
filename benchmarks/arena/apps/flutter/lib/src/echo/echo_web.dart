import 'dart:js_interop';
import 'dart:typed_data';

import 'echo_module.dart';

/// `globalThis.ArenaEcho`, defined in `web/index.html`: the browser's
/// platform language is JavaScript, so that is where the module lives.
@JS('ArenaEcho.echo')
external JSPromise<JSAny?> _echo([JSAny? value]);

@JS('ArenaEcho.addOne')
external JSPromise<JSNumber> _addOne(JSNumber value);

@JS('ArenaEcho.echoSync')
external JSAny? _echoSync([JSAny? value]);

@JS('ArenaEcho.addOneSync')
external JSNumber _addOneSync(JSNumber value);

AsyncEcho createAsyncEcho() => const _PromiseEcho();

SyncEcho createSyncEcho() => const _JsSyncEcho();

/// Awaits a JavaScript Promise for every call.
final class _PromiseEcho implements AsyncEcho {
  const _PromiseEcho();

  @override
  String get description =>
      'async: JS interop to a Promise-returning JavaScript module (the web '
      'has no MethodChannel host); bytes travel as a Uint8Array; the '
      'browser coarsens Stopwatch (performance.now) to 100 us without '
      'cross-origin isolation, so per-call latencies are quantised';

  @override
  Future<void> none() => _echo().toDart;

  @override
  Future<int?> addOne(int value) async =>
      (await _addOne(value.toJS).toDart).toDartInt;

  @override
  Future<String?> echoString(String value) async =>
      ((await _echo(value.toJS).toDart)! as JSString).toDart;

  @override
  Future<Uint8List?> echoBytes(Uint8List value) async =>
      ((await _echo(value.toJS).toDart)! as JSUint8Array).toDart;

  @override
  Future<Map<Object?, Object?>?> echoObject(Map<String, int> value) async =>
      (await _echo(value.jsify()).toDart).dartify() as Map<Object?, Object?>?;
}

/// Calls the same JavaScript module synchronously: the web's equivalent of a
/// `dart:ffi` call, since there is no `dart:ffi` in the browser.
final class _JsSyncEcho implements SyncEcho {
  const _JsSyncEcho();

  @override
  String get description =>
      'sync: synchronous JS interop calls (no dart:ffi on the web); the object '
      'is converted with jsify and dartify';

  @override
  void none() => _echoSync();

  @override
  int addOne(int value) => _addOneSync(value.toJS).toDartInt;

  @override
  String echoString(String value) =>
      (_echoSync(value.toJS)! as JSString).toDart;

  @override
  Uint8List echoBytes(Uint8List value) =>
      (_echoSync(value.toJS)! as JSUint8Array).toDart;

  @override
  Map<String, int> echoObject(Map<String, int> value) {
    final echoed = _echoSync(value.jsify()).dartify()! as Map<Object?, Object?>;
    return {
      for (final MapEntry(:key, :value) in echoed.entries)
        key! as String: (value! as num).toInt(),
    };
  }
}
