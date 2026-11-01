// The live scenario for the generated Dart client; see `sdk_live.rs`.
// Runs as `bin/client.dart` inside the generated `sdk_live` package.

import 'dart:async';
import 'dart:io';

import 'package:sdk_live/sdk_live.dart';

String env(String name) {
  final value = Platform.environment[name];
  if (value == null) {
    throw StateError('$name is not set');
  }
  return value;
}

final String serverUrl = env('SDK_LIVE_URL');
final String prefix = env('SDK_LIVE_PREFIX');

void check(bool condition, String message) {
  if (!condition) {
    throw StateError(message);
  }
}

/// Completes when [test] holds, checking after every event on [changes].
Future<void> waitFor(Stream<void> changes, bool Function() test, String what) {
  if (test()) {
    return Future<void>.value();
  }
  return changes.firstWhere((_) => test()).timeout(
        const Duration(seconds: 20),
        onTimeout: () => throw TimeoutException('waited too long for $what'),
      );
}

/// One person's client, holding the latest value of their subscription.
class Person {
  Person(this.token)
      : api = Client(
          serverUrl: serverUrl,
          live: LiveOptions(
            token: () => token,
            minBackoff: const Duration(milliseconds: 50),
            maxBackoff: const Duration(milliseconds: 200),
          ),
        ) {
    api.live.statuses.listen((status) => statuses.add(status.state));
    subscription = api.sdkNotes.watch(prefix: prefix).listen(
      (value) {
        notes = value;
        _changes.add(null);
      },
      onError: (Object error) => stderr.writeln('$token: $error'),
    );
  }

  final String token;
  final Client api;
  final List<LiveState> statuses = <LiveState>[];
  final StreamController<void> _changes = StreamController<void>.broadcast();
  late final StreamSubscription<List<SdkNote>> subscription;
  List<SdkNote>? notes;

  int has(String body) => notes?.where((note) => note.body == body).length ?? 0;

  Future<void> until(bool Function() test, String what) =>
      waitFor(_changes.stream, test, '$token: $what');
}

Future<void> scenario() async {
  final ada = Person('ada');
  final bob = Person('bob');

  await ada.until(() => ada.notes != null, 'the first value');
  await bob.until(() => bob.notes != null, 'the first value');
  check(ada.notes!.isEmpty, 'a fresh database has no notes');
  print('ok initial');

  // Read-your-writes: when the call completes, Bob's own subscription has it.
  final first = await bob.api.addSdkNote.live(body: '${prefix}one');
  check(first.isNotEmpty, 'the mutation returns the id');
  check(bob.has('${prefix}one') == 1, "Bob sees his own write: ${bob.notes}");
  print('ok read-your-writes');

  // Ada's subscription receives it as a patch.
  await ada.until(() => ada.has('${prefix}one') == 1, "Bob's note");
  print('ok patch');

  // The proxy lets this through, then cuts the connection before the answer.
  final dropped = await ada.api.addSdkNote.live(body: '${prefix}drop-me');
  check(dropped.isNotEmpty, 'the re-sent mutation answers');
  await ada.until(() => ada.has('${prefix}drop-me') == 1, 'the dropped note');
  check(
    ada.statuses.contains(LiveState.reconnecting),
    'Ada reconnected: ${ada.statuses.map((state) => state.name).join(', ')}',
  );
  check(ada.has('${prefix}drop-me') == 1, 'the re-sent mutation ran once');
  print('ok reconnect');

  await bob.api.addSdkNote.live(body: '${prefix}two');
  await ada.until(
      () => ada.has('${prefix}two') == 1, 'a note after the reconnect');
  print('ok resumed');

  // Channels: presence from both, and Bob's signal reaches Ada.
  final signals = <ChannelSignal<SdkSignal>>[];
  var adaMembers = <PresenceMember<SdkPresence>>[];
  var bobMembers = <PresenceMember<SdkPresence>>[];
  final changed = StreamController<void>.broadcast();
  final adaRoom = ada.api.sdkRoom
      .join(SdkRoom(room: prefix), presence: SdkPresence(name: 'Ada'));
  adaRoom.presence.listen((members) {
    adaMembers = members;
    changed.add(null);
  });
  adaRoom.signals.listen((signal) {
    signals.add(signal);
    changed.add(null);
  });
  adaRoom.errors.listen((error) => stderr.writeln('ada join: $error'));
  final bobRoom = bob.api.sdkRoom
      .join(SdkRoom(room: prefix), presence: SdkPresence(name: 'Bob'));
  bobRoom.presence.listen((members) {
    bobMembers = members;
    changed.add(null);
  });
  String names(List<PresenceMember<SdkPresence>> members) =>
      (members.map((member) => '${member.id}:${member.state.name}').toList()
            ..sort())
          .join(',');
  await waitFor(
    changed.stream,
    () =>
        names(adaMembers) == 'ada:Ada,bob:Bob' &&
        names(bobMembers) == 'ada:Ada,bob:Bob',
    'both in the room',
  );
  bobRoom.signal(SdkSignal(text: 'hi'));
  await waitFor(changed.stream, () => signals.isNotEmpty, "Bob's signal");
  check(
    signals[0].message.text == 'hi' && signals[0].from == 'bob',
    'the signal: $signals',
  );
  adaRoom.leave();
  bobRoom.leave();
  print('ok channel');

  await ada.subscription.cancel();
  await bob.subscription.cancel();
  ada.api.live.close();
  bob.api.live.close();
}

Future<void> main() async {
  final budget = Timer(const Duration(seconds: 60), () {
    stderr.writeln('timed out');
    exit(1);
  });
  var code = 0;
  try {
    await scenario();
  } catch (error, stack) {
    stderr
      ..writeln(error)
      ..writeln(stack);
    code = 1;
  }
  budget.cancel();
  await stdout.flush();
  await stderr.flush();
  // The sockets' closing handshakes needn't finish.
  exit(code);
}
