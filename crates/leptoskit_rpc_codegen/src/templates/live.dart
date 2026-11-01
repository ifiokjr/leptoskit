// The live protocol runtime: one WebSocket to the server's `/live`, carrying
// subscriptions, mutations and channels, kept up across reconnects.
//
// It opens the socket with `package:web_socket_channel`, which works in
// Flutter on every platform, in the browser, and in Dart on the server or the
// command line; pass `LiveOptions.connect` to open it some other way.

import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:web_socket_channel/web_socket_channel.dart';

/// The WebSocket subprotocol the server speaks.
const String liveSubprotocol = 'leptoskit.live.v1.json';

/// A failure the server reported over the live connection.
class LiveError implements Exception {
  /// Creates an error with the HTTP [status] the same failure would carry.
  LiveError(this.status, this.message);

  /// The HTTP status the same failure would carry; `0` when the connection
  /// closed or a value didn't decode.
  final int status;

  /// The human-readable message.
  final String message;

  @override
  String toString() => 'LiveError($status): $message';
}

/// Which state the connection is in.
enum LiveState {
  /// Opening the socket.
  connecting,

  /// Open, and receiving changes.
  live,

  /// Lost, and waiting to try again.
  reconnecting,

  /// Closed, or not needed yet.
  offline,
}

/// Where the connection is, for a "reconnecting…" banner.
class LiveStatus {
  const LiveStatus._(
    this.state, {
    this.attempt = 0,
    this.nextIn,
    this.version,
    this.connection,
  });

  /// Which state the connection is in.
  final LiveState state;

  /// How many times in a row the connection has been lost.
  final int attempt;

  /// How long until the next try, while [LiveState.reconnecting].
  final Duration? nextIn;

  /// The version the server started from, while [LiveState.live].
  final int? version;

  /// The server's name for the connection, for logs, while [LiveState.live].
  final String? connection;

  @override
  String toString() => switch (state) {
        LiveState.connecting => 'connecting (attempt $attempt)',
        LiveState.live => 'live at version $version',
        LiveState.reconnecting =>
          'reconnecting in ${nextIn?.inMilliseconds} ms (attempt $attempt)',
        LiveState.offline => 'offline',
      };
}

/// Someone in a channel, and the presence they joined with.
class PresenceMember<P> {
  /// Creates a member.
  const PresenceMember(this.id, this.state);

  /// Who: the caller id the server knows them by.
  final String id;

  /// What they show the others.
  final P state;

  @override
  String toString() => 'PresenceMember($id, $state)';
}

/// A signal another member of a channel sent.
class ChannelSignal<M> {
  /// Creates a signal.
  const ChannelSignal(this.message, this.from);

  /// What they sent.
  final M message;

  /// Who sent it, when the server can say.
  final String? from;

  @override
  String toString() => 'ChannelSignal($from, $message)';
}

/// How the live connection behaves.
class LiveOptions {
  /// Creates the options; every one has a default.
  const LiveOptions({
    this.token,
    this.onAuthRefused,
    this.connect,
    this.readYourWrites = const Duration(seconds: 2),
    this.minBackoff = const Duration(milliseconds: 250),
    this.maxBackoff = const Duration(seconds: 30),
  });

  /// Returns the bearer token the connection signs in with, such as a native
  /// session's access token; called on every connect, and again when the
  /// server says the token is about to expire. A browser on the app's own
  /// origin needs none: its session cookie rides the upgrade.
  final FutureOr<String?> Function()? token;

  /// Called when the server refuses the token.
  final void Function()? onAuthRefused;

  /// Opens the socket; defaults to [WebSocketChannel.connect].
  final WebSocketChannel Function(Uri url, List<String> protocols)? connect;

  /// How long a mutation over the socket waits for its own write to reach the
  /// subscriptions before it resolves anyway.
  final Duration readYourWrites;

  /// The first reconnect delay's upper bound.
  final Duration minBackoff;

  /// The longest reconnect delay.
  final Duration maxBackoff;
}

/// A joined channel.
class ChannelHandle<M, P> {
  ChannelHandle._(this._connection, this._key, this._joined);

  final LiveConnection _connection;
  final String _key;
  final _Joined<M, P> _joined;

  /// Everyone in the channel, as last reported.
  List<PresenceMember<P>> get members => _joined.members;

  /// Each signal another member sends.
  Stream<ChannelSignal<M>> get signals => _joined.signals.stream;

  /// Everyone in the channel, after each join and leave; a new listener gets
  /// the last list first.
  Stream<List<PresenceMember<P>>> get presence =>
      _withLatest(_joined.presence.stream, () => _joined.members,
          isEmpty: (members) => members.isEmpty);

  /// Why the server refused the join, or a message that didn't decode.
  Stream<LiveError> get errors => _joined.errors.stream;

  bool get _current => _connection._channels[_key] == _joined;

  /// Sends a signal to everyone else in the channel, at most once.
  void signal(M message) {
    if (_current) {
      _connection._send(<String, Object?>{
        'type': 'signal',
        'channel': _key,
        'message': _joined.encodeMessage(message),
      });
    }
  }

  /// Changes what this connection shows the others.
  void setPresence(P state) {
    if (_current) {
      _joined.state = _joined.encodePresence(state);
      _connection._send(_joined.frame(_key));
    }
  }

  /// Leaves the channel and closes its streams.
  void leave() {
    if (_current) {
      _connection._channels.remove(_key);
      _connection._send(<String, Object?>{'type': 'leave', 'channel': _key});
      _joined.shutDown();
    }
  }
}

/// The name of a channel instance: its join arguments as compact JSON, with the
/// fields in the order the server declares them, as the app's own client
/// writes it, so every client of the same instance meets in one room.
String channelKey(List<String> fields, Object? args) {
  if (args is! Map) {
    return jsonEncode(args);
  }
  final ordered = <String, Object?>{};
  for (final field in fields) {
    ordered[field] = args[field];
  }
  for (final entry in args.entries) {
    final field = '${entry.key}';
    if (!ordered.containsKey(field)) {
      ordered[field] = entry.value;
    }
  }
  return jsonEncode(ordered);
}

/// The live URL for a server origin, such as `https://api.example.com`; with
/// none, the page's own origin in a browser.
Uri liveUrl(String? serverUrl, String path) {
  if (serverUrl == null || serverUrl.isEmpty) {
    final base = Uri.base;
    if (base.scheme != 'http' && base.scheme != 'https') {
      throw LiveError(
          0, 'a live connection needs a server URL outside a browser');
    }
    return Uri(
      scheme: base.scheme == 'https' ? 'wss' : 'ws',
      host: base.host,
      port: base.hasPort ? base.port : null,
      path: path,
    );
  }
  final origin = serverUrl.replaceFirst(RegExp(r'/+$'), '');
  return Uri.parse(origin.replaceFirst(RegExp('^http'), 'ws') + path);
}

/// One live connection, opened on first use and kept up until [close].
class LiveConnection {
  /// Creates a connection to [url]; nothing opens until it is used.
  LiveConnection(this.url, [this.options = const LiveOptions()]);

  /// The socket's URL.
  final Uri url;

  /// How the connection signs in and reconnects.
  final LiveOptions options;

  WebSocketChannel? _socket;
  StreamSubscription<Object?>? _listening;
  bool _ready = false;
  bool _closed = false;
  LiveStatus _status = const LiveStatus._(LiveState.offline);
  final StreamController<LiveStatus> _statuses =
      StreamController<LiveStatus>.broadcast();
  int _nextId = 1;
  final Map<int, _Subscription> _subscriptions = <int, _Subscription>{};
  final Map<int, _Pending> _pending = <int, _Pending>{};
  final List<_Waiter> _waiters = <_Waiter>[];
  final Map<String, _Channel> _channels = <String, _Channel>{};
  int _version = 0;
  int _attempt = 0;
  Timer? _retry;
  Timer? _heartbeat;
  Duration _heartbeatInterval = const Duration(seconds: 30);
  DateTime _lastHeard = DateTime.now();
  Duration? _reconnectAfter;

  /// Where the connection is now.
  LiveStatus get status => _status;

  /// Every status change; a new listener gets the current status first, so it
  /// suits a `StreamBuilder` for a connection banner.
  Stream<LiveStatus> get statuses =>
      _withLatest(_statuses.stream, () => _status);

  /// The last version the connection applied.
  int get version => _version;

  /// Watches the query [name]: the stream gets the first value and each new
  /// one, decoded by [decode]. A failure arrives as a [LiveError] error event;
  /// the subscription stays, and recovers on the next change. Every listener
  /// gets its own subscription, which ends when the listener cancels.
  Stream<T> watch<T>(
      String name, Object? args, T Function(Object? json) decode) {
    return Stream<T>.multi((controller) {
      if (_closed) {
        controller.addError(LiveError(0, 'the live connection is closed'));
        controller.close();
        return;
      }
      final id = _nextId++;
      final query = _present(args);
      // Values go out synchronously, before a mutation waiting on the same
      // transition resolves, so its caller already sees its own write.
      _subscriptions[id] = _Subscription(
        query: name,
        args: query,
        onValue: (value) {
          final T decoded;
          try {
            decoded = decode(value);
          } catch (error) {
            controller.addErrorSync(
                LiveError(0, 'the result of `$name` did not decode: $error'));
            return;
          }
          controller.addSync(decoded);
        },
        onError: controller.addErrorSync,
        onClose: controller.close,
      );
      _send(<String, Object?>{
        'type': 'sub',
        'id': id,
        'query': name,
        'args': query
      });
      _ensure();
      controller.onCancel = () {
        if (_subscriptions.remove(id) != null) {
          _send(<String, Object?>{'type': 'unsub', 'id': id});
        }
      };
    });
  }

  /// Runs the mutation [name] over the socket. It completes once the
  /// connection's subscriptions show the write (read-your-writes), or after
  /// [LiveOptions.readYourWrites] if no change to them arrives. A call made
  /// while the connection is down waits for it, and one whose answer is lost
  /// is sent again with the same request id and key, so it runs once.
  Future<Object?> mutate(String name, Object? args, {String? idempotencyKey}) {
    if (_closed) {
      return Future<Object?>.error(
          LiveError(0, 'the live connection is closed'));
    }
    final request = _nextId++;
    final frame = <String, Object?>{
      'type': 'mutate',
      'request': request,
      'name': name,
      'args': _present(args),
      'key': idempotencyKey ?? _randomKey(),
    };
    final completer = Completer<Object?>();
    _pending[request] = _Pending(frame, completer);
    _send(frame);
    _ensure();
    return completer.future;
  }

  /// Joins the channel [name] with [args]: signals and presence arrive on the
  /// handle's streams, and the join is made again after a reconnect, until
  /// [ChannelHandle.leave]. [fields] are the join arguments' names in the
  /// order the server declares them, which name the channel instance.
  ChannelHandle<M, P> join<M, P>(
    String name, {
    required List<String> fields,
    required Object? args,
    P? presence,
    required M Function(Object? json) decodeMessage,
    required Object? Function(M message) encodeMessage,
    required P Function(Object? json) decodePresence,
    required Object? Function(P state) encodePresence,
  }) {
    if (_closed) {
      throw LiveError(0, 'the live connection is closed');
    }
    final key = channelKey(fields, args);
    final joined = _Joined<M, P>(
      name: name,
      args: _present(args),
      state: presence == null ? null : encodePresence(presence),
      decodeMessage: decodeMessage,
      encodeMessage: encodeMessage,
      decodePresence: decodePresence,
      encodePresence: encodePresence,
    );
    _channels.remove(key)?.shutDown();
    _channels[key] = joined;
    _send(joined.frame(key));
    _ensure();
    return ChannelHandle<M, P>._(this, key, joined);
  }

  /// Sends the token again, such as after signing in.
  Future<void> reauthenticate() async {
    final token = await options.token?.call();
    if (token != null && token.isNotEmpty) {
      _send(<String, Object?>{'type': 'auth', 'token': token});
    }
  }

  /// Closes the connection for good: pending mutations fail, and every
  /// stream ends.
  void close() {
    if (_closed) {
      return;
    }
    _closed = true;
    _clearTimers();
    final socket = _socket;
    _socket = null;
    _ready = false;
    _listening?.cancel();
    _listening = null;
    if (socket != null) {
      _quietly(socket.sink.close(1000, 'closed'));
    }
    for (final pending in _pending.values) {
      pending.completer
          .completeError(LiveError(0, 'the live connection closed'));
    }
    _pending.clear();
    for (final waiter in _waiters) {
      waiter.finish();
    }
    _waiters.clear();
    for (final subscription in _subscriptions.values) {
      subscription.onClose();
    }
    _subscriptions.clear();
    for (final joined in _channels.values) {
      joined.shutDown();
    }
    _channels.clear();
    _setStatus(const LiveStatus._(LiveState.offline));
    _statuses.close();
  }

  void _send(Map<String, Object?> frame) {
    final socket = _socket;
    if (_ready && socket != null) {
      _write(socket, frame);
    }
  }

  void _write(WebSocketChannel socket, Map<String, Object?> frame) {
    try {
      socket.sink.add(jsonEncode(frame));
    } catch (_) {
      // The socket is closing; its end reconnects.
    }
  }

  bool get _busy =>
      _subscriptions.isNotEmpty || _pending.isNotEmpty || _channels.isNotEmpty;

  void _ensure() {
    if (_socket == null && _retry == null && !_closed) {
      _open();
    }
  }

  void _open() {
    _setStatus(LiveStatus._(LiveState.connecting, attempt: _attempt));
    final WebSocketChannel socket;
    try {
      final connect = options.connect ??
          ((Uri url, List<String> protocols) =>
              WebSocketChannel.connect(url, protocols: protocols));
      socket = connect(url, const <String>[liveSubprotocol]);
    } catch (_) {
      _lost();
      return;
    }
    _socket = socket;
    _lastHeard = DateTime.now();
    _listening = socket.stream.listen(
      (Object? data) {
        if (_socket != socket) {
          return;
        }
        _lastHeard = DateTime.now();
        final Object? frame;
        try {
          frame = jsonDecode(
              data is String ? data : utf8.decode(data as List<int>));
        } catch (_) {
          return;
        }
        if (frame is Map<String, dynamic>) {
          _receive(frame);
        }
      },
      onError: (Object _) {
        // The stream ends next.
      },
      onDone: () {
        if (_socket == socket) {
          _lost();
        }
      },
    );
    socket.ready.then(
      (_) => _greet(socket),
      onError: (Object _) {
        if (_socket == socket) {
          _lost();
        }
      },
    );
  }

  /// Says hello, signs in, and sends everything the connection holds.
  Future<void> _greet(WebSocketChannel socket) async {
    if (_socket != socket) {
      return;
    }
    _write(
      socket,
      _version > 0
          ? <String, Object?>{
              'type': 'hello',
              'resume': <String, Object?>{'version': _version},
            }
          : <String, Object?>{'type': 'hello'},
    );
    String? token;
    try {
      token = await options.token?.call();
    } catch (_) {
      token = null;
    }
    if (_socket != socket) {
      return;
    }
    if (token != null && token.isNotEmpty) {
      _write(socket, <String, Object?>{'type': 'auth', 'token': token});
    }
    for (final entry in _subscriptions.entries) {
      _write(socket, <String, Object?>{
        'type': 'sub',
        'id': entry.key,
        'query': entry.value.query,
        'args': entry.value.args,
      });
    }
    for (final entry in _channels.entries) {
      _write(socket, entry.value.frame(entry.key));
    }
    // The same request id and key: the server's receipt answers a call that
    // already ran with its first result.
    for (final pending in _pending.values) {
      _write(socket, pending.frame);
    }
    _ready = true;
  }

  void _receive(Map<String, dynamic> frame) {
    switch (frame['type']) {
      case 'welcome':
        _attempt = 0;
        _version = _int(frame['version'], _version);
        _startHeartbeat(_int(frame['heartbeat_seconds'], 30));
        _setStatus(LiveStatus._(
          LiveState.live,
          version: _version,
          connection: '${frame['connection'] ?? ''}',
        ));
      case 'transition':
        _transition(frame);
      case 'done':
        _done(frame);
      case 'auth_status':
        if (frame['refused'] == true) {
          options.onAuthRefused?.call();
        } else if (frame['expiring'] == true) {
          _quietly(reauthenticate());
        }
      case 'reconnect':
        _reconnectAfter = Duration(milliseconds: _int(frame['after_ms'], 0));
      case 'signal':
        final from = frame['from'];
        _channels['${frame['channel']}']
            ?.receiveSignal(frame['message'], from is String ? from : null);
      case 'presence':
        final members = frame['members'];
        if (members is List) {
          _channels['${frame['channel']}']?.receivePresence(members);
        }
      case 'channel_error':
        _channels['${frame['channel']}']
            ?.receiveError(_wireError(frame['err']));
      default:
        // `pong`, and frames a newer server sends that this runtime doesn't
        // know.
        break;
    }
  }

  /// Applies one transition: every update, then every listener, then the
  /// version.
  void _transition(Map<String, dynamic> frame) {
    final updates = frame['updates'];
    final changed = <(_Subscription, Object?)>[];
    final failed = <(_Subscription, LiveError)>[];
    for (final update in updates is List ? updates : const <Object?>[]) {
      if (update is! Map) {
        continue;
      }
      final subscription = _subscriptions[_int(update['id'], -1)];
      if (subscription == null) {
        continue;
      }
      if (update.containsKey('ok')) {
        changed.add((subscription, update['ok']));
      } else if (update.containsKey('patch')) {
        changed.add(
            (subscription, _applyPatch(subscription.value, update['patch'])));
      } else if (update.containsKey('err')) {
        failed.add((subscription, _wireError(update['err'])));
      }
    }
    final to = _int(frame['to'], _version);
    _version = math.max(_version, to);
    for (final (subscription, value) in changed) {
      subscription.value = value;
    }
    for (final (subscription, value) in changed) {
      subscription.onValue(value);
    }
    for (final (subscription, error) in failed) {
      subscription.onError(error);
    }
    _reached(to);
  }

  void _done(Map<String, dynamic> frame) {
    final request = _int(frame['request'], 0);
    final version = _int(frame['version'], 0);
    final pending = _pending.remove(request);
    if (pending != null) {
      final err = frame['err'];
      if (err != null) {
        pending.completer.completeError(_wireError(err));
        return;
      }
      final value = frame['ok'];
      if (_subscriptions.isEmpty || _version >= version) {
        pending.completer.complete(value);
        return;
      }
      late final _Waiter waiter;
      waiter = _Waiter(
        version,
        value,
        pending.completer,
        Timer(options.readYourWrites, () {
          _waiters.remove(waiter);
          waiter.finish();
        }),
      );
      _waiters.add(waiter);
      return;
    }
    final subscription = _subscriptions[request];
    if (subscription == null) {
      return;
    }
    final err = frame['err'];
    if (err != null) {
      subscription.onError(_wireError(err));
      return;
    }
    subscription.value = frame['ok'];
    subscription.onValue(subscription.value);
    // A fresh first value reflects every write the server had committed.
    _reached(version);
  }

  void _reached(int version) {
    final ready =
        _waiters.where((waiter) => waiter.version <= version).toList();
    if (ready.isEmpty) {
      return;
    }
    _waiters.removeWhere((waiter) => waiter.version <= version);
    for (final waiter in ready) {
      waiter.finish();
    }
  }

  void _startHeartbeat(int seconds) {
    _heartbeat?.cancel();
    _heartbeatInterval = Duration(seconds: math.max(1, seconds));
    _heartbeat = Timer.periodic(_heartbeatInterval, (_) {
      final socket = _socket;
      if (socket == null) {
        return;
      }
      final silence = DateTime.now().difference(_lastHeard);
      if (silence > _heartbeatInterval * 2 + const Duration(seconds: 5)) {
        // Two missed heartbeats: the connection is gone without saying so.
        _quietly(socket.sink.close(4000, 'no heartbeat'));
        _lost();
        return;
      }
      _send(<String, Object?>{'type': 'ping'});
    });
  }

  void _clearTimers() {
    _retry?.cancel();
    _retry = null;
    _heartbeat?.cancel();
    _heartbeat = null;
  }

  /// The socket closed: try again after a backoff, or when the server said.
  void _lost() {
    _socket = null;
    _ready = false;
    _listening?.cancel();
    _listening = null;
    _clearTimers();
    if (_closed) {
      return;
    }
    if (!_busy) {
      _reconnectAfter = null;
      _setStatus(const LiveStatus._(LiveState.offline));
      return;
    }
    final Duration delay;
    final after = _reconnectAfter;
    if (after != null) {
      delay = after;
      _reconnectAfter = null;
    } else {
      final ceiling = math.min(
        options.maxBackoff.inMicroseconds.toDouble(),
        options.minBackoff.inMicroseconds *
            math.pow(2, math.min(_attempt, 30)).toDouble(),
      );
      delay = Duration(microseconds: (_random.nextDouble() * ceiling).floor());
    }
    _attempt += 1;
    _setStatus(
        LiveStatus._(LiveState.reconnecting, attempt: _attempt, nextIn: delay));
    _retry = Timer(delay, () {
      _retry = null;
      if (!_closed) {
        _open();
      }
    });
  }

  void _setStatus(LiveStatus status) {
    _status = status;
    if (!_statuses.isClosed) {
      _statuses.add(status);
    }
  }
}

class _Subscription {
  _Subscription({
    required this.query,
    required this.args,
    required this.onValue,
    required this.onError,
    required this.onClose,
  });

  final String query;
  final Object? args;
  final void Function(Object? value) onValue;
  final void Function(LiveError error) onError;
  final void Function() onClose;

  /// The last value, as JSON, for the next patch to apply to.
  Object? value;
}

class _Pending {
  _Pending(this.frame, this.completer);

  final Map<String, Object?> frame;
  final Completer<Object?> completer;
}

class _Waiter {
  _Waiter(this.version, this.value, this.completer, this.timer);

  final int version;
  final Object? value;
  final Completer<Object?> completer;
  final Timer timer;

  void finish() {
    timer.cancel();
    if (!completer.isCompleted) {
      completer.complete(value);
    }
  }
}

/// A joined channel, as the connection holds it: everything it does with a
/// frame works on JSON, so it needs no type arguments.
abstract class _Channel {
  Map<String, Object?> frame(String key);
  void receiveSignal(Object? message, String? from);
  void receivePresence(List<Object?> members);
  void receiveError(LiveError error);
  void shutDown();
}

class _Joined<M, P> implements _Channel {
  _Joined({
    required this.name,
    required this.args,
    required this.state,
    required this.decodeMessage,
    required this.encodeMessage,
    required this.decodePresence,
    required this.encodePresence,
  });

  final String name;
  final Object? args;
  Object? state;
  final M Function(Object? json) decodeMessage;
  final Object? Function(M message) encodeMessage;
  final P Function(Object? json) decodePresence;
  final Object? Function(P state) encodePresence;
  List<PresenceMember<P>> members = <PresenceMember<P>>[];
  final StreamController<ChannelSignal<M>> signals =
      StreamController<ChannelSignal<M>>.broadcast();
  final StreamController<List<PresenceMember<P>>> presence =
      StreamController<List<PresenceMember<P>>>.broadcast();
  final StreamController<LiveError> errors =
      StreamController<LiveError>.broadcast();

  @override
  Map<String, Object?> frame(String key) => <String, Object?>{
        'type': 'join',
        'channel': key,
        'name': name,
        'args': args,
        'state': state,
      };

  @override
  void receiveSignal(Object? message, String? from) {
    final M decoded;
    try {
      decoded = decodeMessage(message);
    } catch (error) {
      _report(LiveError(0, 'a signal on `$name` did not decode: $error'));
      return;
    }
    if (!signals.isClosed) {
      signals.add(ChannelSignal<M>(decoded, from));
    }
  }

  @override
  void receivePresence(List<Object?> members) {
    final decoded = <PresenceMember<P>>[];
    for (final member in members) {
      if (member is! Map) {
        continue;
      }
      try {
        decoded.add(PresenceMember<P>(
            '${member['id']}', decodePresence(member['state'])));
      } catch (error) {
        // One member's odd presence doesn't hide the rest.
        _report(LiveError(0,
            'the presence of `${member['id']}` on `$name` did not decode: $error'));
      }
    }
    this.members = List<PresenceMember<P>>.unmodifiable(decoded);
    if (!presence.isClosed) {
      presence.add(this.members);
    }
  }

  @override
  void receiveError(LiveError error) => _report(error);

  void _report(LiveError error) {
    if (!errors.isClosed) {
      errors.add(error);
    }
  }

  @override
  void shutDown() {
    signals.close();
    presence.close();
    errors.close();
  }
}

/// [stream], with the value [latest] returns first for each new listener,
/// unless [isEmpty] says there is nothing to show yet.
Stream<T> _withLatest<T>(Stream<T> stream, T Function() latest,
    {bool Function(T value)? isEmpty}) {
  return Stream<T>.multi((controller) {
    final value = latest();
    if (isEmpty == null || !isEmpty(value)) {
      controller.add(value);
    }
    final subscription = stream.listen(
      controller.add,
      onError: controller.addError,
      onDone: controller.close,
    );
    controller
      ..onPause = subscription.pause
      ..onResume = subscription.resume
      ..onCancel = subscription.cancel;
  });
}

/// Applies a list patch or a page's row and metadata changes together.
Object? _applyPatch(Object? value, Object? patch) {
  if (patch is! Map) {
    return value;
  }
  final field = patch['field'];
  if (field is String) {
    if (value is! Map) {
      return value;
    }
    final fields = Map<String, Object?>.from(value);
    final changed = patch['set'];
    if (changed is Map) {
      fields.addAll(Map<String, Object?>.from(changed));
    }
    if (fields.containsKey(field)) {
      fields[field] = _applyRows(fields[field], patch);
    }
    return fields;
  }
  return _applyRows(value, patch);
}

/// Removes rows first, then replaces matching ids or inserts new rows.
Object? _applyRows(Object? value, Map<dynamic, dynamic> patch) {
  if (value is! List) {
    return value;
  }
  final removals = patch['remove'];
  final remove = removals is List ? removals.toSet() : const <Object?>{};
  final items = <Object?>[
    for (final item in value)
      if (!(item is Map && item['id'] is String && remove.contains(item['id'])))
        item,
  ];
  final upserts = patch['upsert'];
  for (final upsert in upserts is List ? upserts : const <Object?>[]) {
    if (upsert is! Map) {
      continue;
    }
    final at = _int(upsert['at'], items.length).clamp(0, items.length).toInt();
    final row = upsert['value'];
    final id = row is Map ? row['id'] : null;
    final previous = at < items.length ? items[at] : null;
    if (id is String && previous is Map && previous['id'] == id) {
      items[at] = row;
    } else {
      items.insert(at, row);
    }
  }
  return items;
}

LiveError _wireError(Object? err) {
  final error = err is Map ? err : const <String, Object?>{};
  return LiveError(
      _int(error['status'], 500), '${error['message'] ?? 'the server failed'}');
}

int _int(Object? value, int fallback) =>
    value is num && value.isFinite ? value.toInt() : fallback;

/// The arguments without absent optional fields, as the HTTP calls send them.
Object? _present(Object? args) {
  if (args == null) {
    return const <String, Object?>{};
  }
  if (args is! Map) {
    return args;
  }
  return <String, Object?>{
    for (final entry in args.entries)
      if (entry.value != null) '${entry.key}': entry.value,
  };
}

/// Ignores a future's outcome, such as closing a socket that already closed.
void _quietly(Future<void> future) {
  future.then<void>((_) {}, onError: (Object _) {});
}

final math.Random _random = () {
  try {
    return math.Random.secure();
  } on UnsupportedError {
    return math.Random();
  }
}();

/// A random version 4 UUID, for an idempotency key.
String _randomKey() {
  final bytes = List<int>.generate(16, (_) => _random.nextInt(256));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  final hex =
      bytes.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join();
  return '${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-'
      '${hex.substring(16, 20)}-${hex.substring(20)}';
}
