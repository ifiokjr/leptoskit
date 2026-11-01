"""The live protocol runtime: one WebSocket to the server's live path, carrying
watched queries, mutations and channels, kept up across reconnects.

It needs the ``websockets`` package (install the ``live`` extra). One asyncio
core speaks the protocol: ``AsyncClient`` runs it on your event loop, and
``Client`` runs it on a background thread with its own loop, so the
synchronous API never needs one.
"""

from __future__ import annotations

import asyncio
import concurrent.futures
import inspect
import json
import logging
import random
import threading
import uuid
from dataclasses import dataclass
from typing import Any, Awaitable, Callable, Generic, Mapping, Optional, TypedDict, TypeVar, Union, cast

LIVE_SUBPROTOCOL = "leptoskit.live.v1.json"
"""The WebSocket subprotocol the server speaks."""

T = TypeVar("T")
M = TypeVar("M")
P = TypeVar("P")

_log = logging.getLogger(__name__)


class LiveError(Exception):
    """A failure the server reported over the live connection.

    ``status`` is the HTTP status the same failure would carry, or ``0`` when the
    connection closed.
    """

    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status
        self.message = message


@dataclass(frozen=True)
class LiveStatus:
    """Where the connection is, for a "reconnecting…" banner.

    ``state`` is ``connecting``, ``live``, ``reconnecting``, or ``offline``.
    ``attempt`` counts the tries since the connection was last live, ``next_in``
    is the seconds until the next one while reconnecting, and ``version`` and
    ``connection`` describe a live connection.
    """

    state: str
    attempt: int = 0
    next_in: float = 0.0
    version: int = 0
    connection: str = ""


class PresenceMember(TypedDict, Generic[P]):
    """Someone in a channel, and the presence they joined with."""

    # Who: the caller id the server knows them by.
    id: str
    # What they show the others.
    state: P


TokenProvider = Callable[[], Union[Optional[str], Awaitable[Optional[str]]]]
"""Returns the bearer token, or an awaitable of it."""


@dataclass
class LiveOptions:
    """How the live connection signs in and reconnects.

    ``token`` returns the bearer token the connection signs in with, such as a
    session's access token; it's called on every connect, and again when the
    server says the token is about to expire, and may be a coroutine function.
    The client's ``headers`` also ride the upgrade request.

    ``read_your_writes`` is how long a live mutation waits for its own write to
    reach the watched queries before it returns anyway, and ``min_backoff`` and
    ``max_backoff`` bound the reconnect delays; all three are in seconds.
    """

    token: Optional[TokenProvider] = None
    on_auth_refused: Optional[Callable[[], None]] = None
    read_your_writes: float = 2.0
    min_backoff: float = 0.25
    max_backoff: float = 30.0


def channel_key(fields: list[str], args: Mapping[str, Any]) -> str:
    """The name of a channel instance: its join arguments as compact JSON.

    The fields come in the order the server declares them, as the app's own
    client writes them, so every client of the same instance meets in one room.
    """
    ordered: dict[str, Any] = {field: args.get(field) for field in fields}
    for field, value in args.items():
        if field not in ordered:
            ordered[field] = value
    return _encode(ordered)


def live_url(server_url: str, path: str) -> str:
    """The live URL for a server origin, such as ``https://api.example.com``."""
    if server_url == "":
        raise LiveError(0, "a live connection needs a server URL; pass one to the client")
    if server_url.startswith("http"):
        return "ws" + server_url[4:] + path
    return server_url + path


class _Subscription:
    def __init__(
        self,
        query: str,
        args: Any,
        on_value: Callable[[Any, int], None],
        on_error: Callable[[LiveError], None],
        on_end: Callable[[], None],
    ) -> None:
        self.query = query
        self.args = args
        self.value: Any = None
        self.on_value = on_value
        self.on_error = on_error
        self.on_end = on_end


class _Pending:
    def __init__(self, frame: dict[str, Any], future: asyncio.Future[Any]) -> None:
        self.frame = frame
        self.future = future


class _Waiter:
    def __init__(self, version: int, value: Any, future: asyncio.Future[Any]) -> None:
        self.version = version
        self.value = value
        self.future = future
        self.timer: Optional[asyncio.TimerHandle] = None


class _Joined:
    def __init__(
        self,
        name: str,
        args: Any,
        state: Any,
        on_signal: Optional[Callable[[Any, Optional[str]], None]],
        on_presence: Optional[Callable[[list[Any]], None]],
        on_error: Optional[Callable[[LiveError], None]],
    ) -> None:
        self.name = name
        self.args = args
        self.state = state
        self.members: list[Any] = []
        self.on_signal = on_signal
        self.on_presence = on_presence
        self.on_error = on_error


class _Core:
    """The protocol, on one event loop: every method runs on ``loop``."""

    def __init__(self, url: str, headers: Mapping[str, str], options: LiveOptions) -> None:
        self.url = url
        self.headers = dict(headers)
        self.options = options
        self.loop: Optional[asyncio.AbstractEventLoop] = None
        self.closed = False
        self.status = LiveStatus("offline")
        self.listeners: list[Callable[[LiveStatus], None]] = []
        self.version = 0
        self._next_id = 1
        self._subscriptions: dict[int, _Subscription] = {}
        self._pending: dict[int, _Pending] = {}
        self._waiters: list[_Waiter] = []
        self._channels: dict[str, _Joined] = {}
        self._attempt = 0
        self._task: Optional[asyncio.Task[None]] = None
        self._socket: Any = None
        self._outbox: Optional[asyncio.Queue[str]] = None
        self._ready = False
        self._heartbeat: Optional[asyncio.Task[None]] = None
        self._heartbeat_seconds = 30.0
        self._last_heard = 0.0
        self._reconnect_after: Optional[float] = None

    # --- What the clients call -------------------------------------------

    def subscribe(
        self,
        name: str,
        args: Any,
        on_value: Callable[[Any, int], None],
        on_error: Callable[[LiveError], None],
        on_end: Callable[[], None],
    ) -> int:
        self._ensure()
        # Subscriptions and mutations share the ids: a subscription's first
        # value arrives as a `done` for its id.
        subscription_id = self._take_id()
        subscription = _Subscription(name, _present(args), on_value, on_error, on_end)
        self._subscriptions[subscription_id] = subscription
        self._send({"type": "sub", "id": subscription_id, "query": name, "args": subscription.args})
        return subscription_id

    def unsubscribe(self, subscription_id: int) -> None:
        subscription = self._subscriptions.pop(subscription_id, None)
        if subscription is not None:
            self._send({"type": "unsub", "id": subscription_id})
            _safely(subscription.on_end)

    def mutate(self, name: str, args: Any, idempotency_key: Optional[str]) -> asyncio.Future[Any]:
        self._ensure()
        assert self.loop is not None
        request = self._take_id()
        frame = {
            "type": "mutate",
            "request": request,
            "name": name,
            "args": _present(args),
            "key": idempotency_key or str(uuid.uuid4()),
        }
        future: asyncio.Future[Any] = self.loop.create_future()
        self._pending[request] = _Pending(frame, future)
        self._send(frame)
        return future

    def join(
        self,
        name: str,
        key: str,
        args: Any,
        state: Any,
        on_signal: Optional[Callable[[Any, Optional[str]], None]],
        on_presence: Optional[Callable[[list[Any]], None]],
        on_error: Optional[Callable[[LiveError], None]],
    ) -> _Joined:
        self._ensure()
        joined = _Joined(name, _present(args), state, on_signal, on_presence, on_error)
        self._channels[key] = joined
        self._send(_join_frame(key, joined))
        return joined

    def signal(self, key: str, joined: _Joined, message: Any) -> None:
        if self._channels.get(key) is joined:
            self._send({"type": "signal", "channel": key, "message": message})

    def set_presence(self, key: str, joined: _Joined, state: Any) -> None:
        if self._channels.get(key) is joined:
            joined.state = state
            self._send(_join_frame(key, joined))

    def leave(self, key: str, joined: _Joined) -> None:
        if self._channels.get(key) is joined:
            del self._channels[key]
            self._send({"type": "leave", "channel": key})

    async def reauthenticate(self) -> None:
        token = await self._token()
        if token:
            self._send({"type": "auth", "token": token})

    async def aclose(self) -> None:
        self.closed = True
        task, self._task = self._task, None
        if task is not None and task is not asyncio.current_task():
            task.cancel()
            try:
                await task
            except BaseException:  # noqa: BLE001 - the task's own end
                pass
        for pending in self._pending.values():
            if not pending.future.done():
                pending.future.set_exception(LiveError(0, "the live connection closed"))
        self._pending.clear()
        for waiter in self._waiters:
            if waiter.timer is not None:
                waiter.timer.cancel()
            _resolve(waiter.future, waiter.value)
        self._waiters.clear()
        subscriptions = list(self._subscriptions.values())
        self._subscriptions.clear()
        self._channels.clear()
        for subscription in subscriptions:
            _safely(subscription.on_end)
        self._set_status(LiveStatus("offline"))

    # --- The connection ----------------------------------------------------

    def _take_id(self) -> int:
        taken = self._next_id
        self._next_id += 1
        return taken

    def _send(self, frame: dict[str, Any]) -> None:
        # Until the greeting is out, the greeting sends everything the
        # connection holds; frames sent meanwhile would arrive twice.
        if self._ready and self._outbox is not None:
            self._outbox.put_nowait(_encode(frame))

    def _busy(self) -> bool:
        return bool(self._subscriptions or self._pending or self._channels)

    def _ensure(self) -> None:
        if self.closed:
            raise LiveError(0, "the live connection is closed")
        _connector()
        if self._task is None:
            assert self.loop is not None
            self._task = self.loop.create_task(self._run())

    async def _run(self) -> None:
        try:
            while not self.closed:
                try:
                    await self._session()
                except asyncio.CancelledError:
                    raise
                except Exception as error:  # noqa: BLE001 - a lost connection
                    _log.debug("the live connection dropped: %s", error)
                if self.closed:
                    return
                if not self._busy():
                    self._reconnect_after = None
                    self._set_status(LiveStatus("offline"))
                    return
                if self._reconnect_after is not None:
                    delay = self._reconnect_after
                    self._reconnect_after = None
                else:
                    # Exponential backoff with full jitter.
                    ceiling = self.options.min_backoff * 2 ** min(self._attempt, 32)
                    delay = random.random() * min(self.options.max_backoff, ceiling)
                self._attempt += 1
                self._set_status(LiveStatus("reconnecting", attempt=self._attempt, next_in=delay))
                await asyncio.sleep(delay)
        finally:
            if self._task is asyncio.current_task():
                self._task = None

    async def _session(self) -> None:
        """One connection, from the upgrade until it closes."""
        assert self.loop is not None
        self._set_status(LiveStatus("connecting", attempt=self._attempt))
        socket = await _connector()(
            self.url,
            subprotocols=[LIVE_SUBPROTOCOL],
            additional_headers=self.headers or None,
            # The runtime pings over the protocol, and the server bounds
            # frames itself.
            ping_interval=None,
            max_size=None,
            close_timeout=2,
        )
        outbox: asyncio.Queue[str] = asyncio.Queue()
        self._socket = socket
        self._outbox = outbox
        self._ready = False
        self._last_heard = self.loop.time()
        writer = self.loop.create_task(_write(socket, outbox))
        reader = self.loop.create_task(self._read(socket))
        try:
            await self._greet(outbox)
            await reader
        finally:
            self._socket = None
            self._outbox = None
            self._ready = False
            if self._heartbeat is not None:
                self._heartbeat.cancel()
                self._heartbeat = None
            writer.cancel()
            reader.cancel()
            try:
                await socket.close()
            except Exception:  # noqa: BLE001 - already gone
                pass

    async def _greet(self, outbox: asyncio.Queue[str]) -> None:
        """Says hello, signs in, and sends everything the connection holds."""
        hello: dict[str, Any] = {"type": "hello"}
        if self.version > 0:
            hello["resume"] = {"version": self.version}
        outbox.put_nowait(_encode(hello))
        try:
            token = await self._token()
        except Exception:  # noqa: BLE001 - connect without a token
            token = None
        if self._outbox is not outbox:
            return
        if token:
            outbox.put_nowait(_encode({"type": "auth", "token": token}))
        for subscription_id, subscription in self._subscriptions.items():
            outbox.put_nowait(
                _encode(
                    {
                        "type": "sub",
                        "id": subscription_id,
                        "query": subscription.query,
                        "args": subscription.args,
                    }
                )
            )
        for key, joined in self._channels.items():
            outbox.put_nowait(_encode(_join_frame(key, joined)))
        # The same request id and key: the server's receipt answers a call
        # that already ran with its first result.
        for pending in self._pending.values():
            outbox.put_nowait(_encode(pending.frame))
        self._ready = True

    async def _token(self) -> Optional[str]:
        provider = self.options.token
        if provider is None:
            return None
        token = provider()
        if inspect.isawaitable(token):
            token = await token
        return token

    async def _read(self, socket: Any) -> None:
        assert self.loop is not None
        async for message in socket:
            self._last_heard = self.loop.time()
            text = message.decode("utf-8", "replace") if isinstance(message, bytes) else message
            try:
                frame = json.loads(text)
            except ValueError:
                continue
            if isinstance(frame, dict):
                self._receive(frame)

    def _receive(self, frame: dict[str, Any]) -> None:
        kind = frame.get("type")
        if kind == "welcome":
            self._attempt = 0
            self.version = _number(frame.get("version"), self.version)
            self._start_heartbeat(_number(frame.get("heartbeat_seconds"), 30))
            self._set_status(
                LiveStatus("live", version=self.version, connection=str(frame.get("connection") or ""))
            )
        elif kind == "transition":
            self._transition(frame)
        elif kind == "done":
            self._done(frame)
        elif kind == "auth_status":
            if frame.get("refused") is True:
                if self.options.on_auth_refused is not None:
                    _safely(self.options.on_auth_refused)
            elif frame.get("expiring") is True:
                assert self.loop is not None
                self.loop.create_task(self.reauthenticate())
        elif kind == "reconnect":
            self._reconnect_after = _number(frame.get("after_ms"), 0) / 1000
        elif kind == "signal":
            joined = self._channels.get(str(frame.get("channel")))
            sender = frame.get("from")
            if joined is not None and joined.on_signal is not None:
                _safely(joined.on_signal, frame.get("message"), sender if isinstance(sender, str) else None)
        elif kind == "presence":
            joined = self._channels.get(str(frame.get("channel")))
            members = frame.get("members")
            if joined is not None and isinstance(members, list):
                joined.members = members
                if joined.on_presence is not None:
                    _safely(joined.on_presence, list(members))
        elif kind == "channel_error":
            joined = self._channels.get(str(frame.get("channel")))
            if joined is not None and joined.on_error is not None:
                _safely(joined.on_error, _wire_error(frame.get("err")))
        # `pong`, and frames a newer server sends that this runtime doesn't
        # know, need nothing.

    def _transition(self, frame: dict[str, Any]) -> None:
        """Applies one transition: every update, then every listener, then the version."""
        updates = frame.get("updates")
        changed: list[tuple[_Subscription, Any]] = []
        failed: list[tuple[_Subscription, LiveError]] = []
        for update in updates if isinstance(updates, list) else []:
            if not isinstance(update, dict):
                continue
            subscription = self._subscriptions.get(_number(update.get("id"), -1))
            if subscription is None:
                continue
            if "ok" in update:
                changed.append((subscription, update["ok"]))
            elif "patch" in update:
                changed.append((subscription, _apply_patch(subscription.value, update["patch"])))
            elif "err" in update:
                failed.append((subscription, _wire_error(update["err"])))
        to = _number(frame.get("to"), self.version)
        self.version = max(self.version, to)
        for subscription, value in changed:
            subscription.value = value
        for subscription, value in changed:
            _safely(subscription.on_value, value, to)
        for subscription, error in failed:
            _safely(subscription.on_error, error)
        self._reached(to)

    def _done(self, frame: dict[str, Any]) -> None:
        request = _number(frame.get("request"), 0)
        version = _number(frame.get("version"), 0)
        error = frame.get("err")
        pending = self._pending.pop(request, None)
        if pending is not None:
            if error is not None:
                if not pending.future.done():
                    pending.future.set_exception(_wire_error(error))
                return
            value = frame.get("ok")
            if not self._subscriptions or self.version >= version:
                _resolve(pending.future, value)
                return
            assert self.loop is not None
            waiter = _Waiter(version, value, pending.future)
            waiter.timer = self.loop.call_later(self.options.read_your_writes, self._expire, waiter)
            self._waiters.append(waiter)
            return
        subscription = self._subscriptions.get(request)
        if subscription is None:
            return
        if error is not None:
            _safely(subscription.on_error, _wire_error(error))
            return
        subscription.value = frame.get("ok")
        _safely(subscription.on_value, subscription.value, version)
        # A fresh first value reflects every write the server had committed.
        self._reached(version)

    def _reached(self, version: int) -> None:
        ready = [waiter for waiter in self._waiters if waiter.version <= version]
        if not ready:
            return
        self._waiters = [waiter for waiter in self._waiters if waiter.version > version]
        for waiter in ready:
            if waiter.timer is not None:
                waiter.timer.cancel()
            _resolve(waiter.future, waiter.value)

    def _expire(self, waiter: _Waiter) -> None:
        if waiter in self._waiters:
            self._waiters.remove(waiter)
            _resolve(waiter.future, waiter.value)

    def _start_heartbeat(self, seconds: float) -> None:
        assert self.loop is not None
        if self._heartbeat is not None:
            self._heartbeat.cancel()
        self._heartbeat_seconds = max(1.0, float(seconds))
        self._heartbeat = self.loop.create_task(self._beat(self._socket))

    async def _beat(self, socket: Any) -> None:
        assert self.loop is not None
        while self._socket is socket:
            await asyncio.sleep(self._heartbeat_seconds)
            if self._socket is not socket:
                return
            if self.loop.time() - self._last_heard > 2 * self._heartbeat_seconds + 5:
                # Two missed heartbeats: the connection is gone without
                # saying so.
                _abort(socket)
                return
            self._send({"type": "ping"})

    def _set_status(self, status: LiveStatus) -> None:
        self.status = status
        for listener in list(self.listeners):
            _safely(listener, status)


class Watch(Generic[T]):
    """A watched query, for the synchronous client.

    Iterate it for the first value and each new one; when several changes arrive
    while you're busy, the next one you get is the newest. A failure is raised as
    a ``LiveError``; the subscription stays and recovers on the next change, so
    iterate again to keep watching. ``value`` is always the latest value, and
    ``close()`` (or leaving a ``with`` block) stops the subscription.
    """

    def __init__(self, connection: LiveConnection, name: str, args: Any) -> None:
        self._connection = connection
        self._changed = threading.Condition()
        self._value: Any = None
        self._has_value = False
        self._version = 0
        self._error: Optional[LiveError] = None
        self._state = 0
        self._seen = 0
        self._ended = False
        core = connection._core
        self._id = connection._call(core.subscribe, name, args, self._on_value, self._on_error, self._on_end)

    @property
    def value(self) -> Optional[T]:
        """The latest value, or ``None`` before the first arrives."""
        return cast(Optional[T], self._value)

    @property
    def version(self) -> int:
        """The version the latest value reflects."""
        return self._version

    @property
    def error(self) -> Optional[LiveError]:
        """The latest failure, until the next value arrives."""
        return self._error

    def wait_for(
        self,
        predicate: Optional[Callable[[T], object]] = None,
        timeout: Optional[float] = None,
    ) -> T:
        """Waits until the latest value satisfies ``predicate`` (or for the first
        value, without one), and returns it.

        Raises ``TimeoutError`` after ``timeout`` seconds, and ``LiveError`` if the
        watch ends first.
        """

        def matched() -> bool:
            return self._has_value and (predicate is None or bool(predicate(self._value)))

        with self._changed:
            if not self._changed.wait_for(lambda: self._ended or matched(), timeout):
                raise TimeoutError("the watched value didn't arrive in time")
            if not matched():
                raise LiveError(0, "the watch ended")
            return cast(T, self._value)

    def close(self) -> None:
        """Stops the subscription."""
        if not self._ended:
            self._connection._call(self._connection._core.unsubscribe, self._id)
            self._on_end()

    def __iter__(self) -> Watch[T]:
        return self

    def __next__(self) -> T:
        with self._changed:
            self._changed.wait_for(lambda: self._state > self._seen or self._ended)
            if self._state == self._seen:
                raise StopIteration
            self._seen = self._state
            if self._error is not None:
                raise self._error
            return cast(T, self._value)

    def __enter__(self) -> Watch[T]:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _on_value(self, value: Any, version: int) -> None:
        with self._changed:
            self._value = value
            self._has_value = True
            self._version = version
            self._error = None
            self._state += 1
            self._changed.notify_all()

    def _on_error(self, error: LiveError) -> None:
        with self._changed:
            self._error = error
            self._state += 1
            self._changed.notify_all()

    def _on_end(self) -> None:
        with self._changed:
            self._ended = True
            self._changed.notify_all()


class AsyncWatch(Generic[T]):
    """A watched query, for the asynchronous client.

    ``async for`` it for the first value and each new one; when several changes
    arrive while you're busy, the next one you get is the newest. A failure is
    raised as a ``LiveError``; the subscription stays and recovers on the next
    change, so iterate again to keep watching. ``value`` is always the latest
    value, and ``close()`` (or leaving an ``async with`` block) stops the
    subscription.
    """

    def __init__(self, core: _Core, name: str, args: Any) -> None:
        self._core = core
        self._wake = asyncio.Event()
        self._value: Any = None
        self._has_value = False
        self._version = 0
        self._error: Optional[LiveError] = None
        self._state = 0
        self._seen = 0
        self._ended = False
        self._id = core.subscribe(name, args, self._on_value, self._on_error, self._on_end)

    @property
    def value(self) -> Optional[T]:
        """The latest value, or ``None`` before the first arrives."""
        return cast(Optional[T], self._value)

    @property
    def version(self) -> int:
        """The version the latest value reflects."""
        return self._version

    @property
    def error(self) -> Optional[LiveError]:
        """The latest failure, until the next value arrives."""
        return self._error

    async def wait_for(
        self,
        predicate: Optional[Callable[[T], object]] = None,
        timeout: Optional[float] = None,
    ) -> T:
        """Waits until the latest value satisfies ``predicate`` (or for the first
        value, without one), and returns it.

        Raises ``TimeoutError`` after ``timeout`` seconds, and ``LiveError`` if the
        watch ends first.
        """

        def matched() -> bool:
            return self._has_value and (predicate is None or bool(predicate(self._value)))

        async def until() -> T:
            while not matched():
                if self._ended:
                    raise LiveError(0, "the watch ended")
                await self._wake.wait()
            return cast(T, self._value)

        return await asyncio.wait_for(until(), timeout)

    def close(self) -> None:
        """Stops the subscription."""
        if not self._ended:
            self._core.unsubscribe(self._id)
            self._on_end()

    def __aiter__(self) -> AsyncWatch[T]:
        return self

    async def __anext__(self) -> T:
        while self._state == self._seen and not self._ended:
            await self._wake.wait()
        if self._state == self._seen:
            raise StopAsyncIteration
        self._seen = self._state
        if self._error is not None:
            raise self._error
        return cast(T, self._value)

    async def __aenter__(self) -> AsyncWatch[T]:
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        self.close()

    def _changed(self) -> None:
        self._state += 1
        wake, self._wake = self._wake, asyncio.Event()
        wake.set()

    def _on_value(self, value: Any, version: int) -> None:
        self._value = value
        self._has_value = True
        self._version = version
        self._error = None
        self._changed()

    def _on_error(self, error: LiveError) -> None:
        self._error = error
        self._changed()

    def _on_end(self) -> None:
        self._ended = True
        self._changed()


class Channel(Generic[M, P]):
    """A joined channel.

    Signals and presence arrive through the callbacks given to ``join``; with
    the synchronous client they run on its background thread. The join is made
    again after a reconnect, until ``leave()`` (or the end of a ``with`` block).
    """

    def __init__(
        self,
        dispatch: Callable[..., Any],
        core: _Core,
        key: str,
        joined: _Joined,
    ) -> None:
        self._dispatch = dispatch
        self._core = core
        self._key = key
        self._joined = joined

    @property
    def key(self) -> str:
        """The channel instance's name: its join arguments as compact JSON."""
        return self._key

    @property
    def members(self) -> list[PresenceMember[P]]:
        """Everyone in the channel, as last reported."""
        return list(self._joined.members)

    def signal(self, message: M) -> None:
        """Sends a signal to everyone else in the channel, at most once."""
        self._dispatch(self._core.signal, self._key, self._joined, message)

    def set_presence(self, state: P) -> None:
        """Changes what this connection shows the others."""
        self._dispatch(self._core.set_presence, self._key, self._joined, state)

    def leave(self) -> None:
        """Leaves the channel."""
        self._dispatch(self._core.leave, self._key, self._joined)

    def __enter__(self) -> Channel[M, P]:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.leave()

    async def __aenter__(self) -> Channel[M, P]:
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        self.leave()


class LiveConnection:
    """The synchronous client's live connection: one WebSocket for every watch,
    live mutation and channel, opened on first use and kept up until ``close()``.

    It runs on a background thread with its own event loop. Read ``status`` or
    pass a listener to ``on_status`` for a connection banner.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: Optional[Mapping[str, str]] = None,
        options: Optional[LiveOptions] = None,
    ) -> None:
        self.url = url
        self._core = _Core(url, headers or {}, options or LiveOptions())
        self._lock = threading.Lock()
        self._thread: Optional[threading.Thread] = None

    @property
    def status(self) -> LiveStatus:
        """Where the connection is now."""
        return self._core.status

    @property
    def version(self) -> int:
        """The last version the connection applied."""
        return self._core.version

    def on_status(self, listener: Callable[[LiveStatus], None]) -> Callable[[], None]:
        """Calls ``listener`` with every status change, on the connection's thread,
        until the returned function is called."""
        self._core.listeners.append(listener)

        def stop() -> None:
            if listener in self._core.listeners:
                self._core.listeners.remove(listener)

        return stop

    def watch(self, name: str, args: Any) -> Watch[Any]:
        """Watches the query ``name``."""
        return Watch(self, name, args)

    def mutate(self, name: str, args: Any, idempotency_key: Optional[str] = None) -> Any:
        """Runs the mutation ``name`` over the socket, and returns its result once
        this connection's watches show the write (read-your-writes), or after
        ``read_your_writes`` seconds if none of them changes.

        A call made while the connection is down waits for it, and one whose
        answer is lost is sent again with the same request id and key, so it runs
        once.
        """
        loop = self._start()
        if self._on_loop():
            raise LiveError(0, "a live callback can't wait for a mutation; use AsyncClient or a thread")

        async def run() -> Any:
            return await self._core.mutate(name, args, idempotency_key)

        return asyncio.run_coroutine_threadsafe(run(), loop).result()

    def join(
        self,
        name: str,
        key: str,
        args: Any,
        *,
        presence: Any = None,
        on_signal: Optional[Callable[[Any, Optional[str]], None]] = None,
        on_presence: Optional[Callable[[list[Any]], None]] = None,
        on_error: Optional[Callable[[LiveError], None]] = None,
    ) -> Channel[Any, Any]:
        """Joins the instance ``key`` of the channel ``name``."""
        core = self._core
        joined = self._call(core.join, name, key, args, presence, on_signal, on_presence, on_error)
        return Channel(self._call, core, key, joined)

    def reauthenticate(self) -> None:
        """Sends the token again, such as after signing in."""
        loop = self._start()
        if self._on_loop():
            loop.create_task(self._core.reauthenticate())
            return
        asyncio.run_coroutine_threadsafe(self._core.reauthenticate(), loop).result()

    def close(self) -> None:
        """Closes the connection for good; pending mutations fail, and watches end."""
        with self._lock:
            loop = self._core.loop
            thread = self._thread
        if loop is None or thread is None or self._core.closed:
            self._core.closed = True
            return
        if self._on_loop():
            loop.create_task(self._core.aclose())
            return
        try:
            asyncio.run_coroutine_threadsafe(self._core.aclose(), loop).result(timeout=10)
        finally:
            loop.call_soon_threadsafe(loop.stop)
            thread.join(timeout=10)
            if not thread.is_alive():
                loop.close()

    def __enter__(self) -> LiveConnection:
        return self

    def __exit__(self, *exc_info: object) -> None:
        self.close()

    def _start(self) -> asyncio.AbstractEventLoop:
        with self._lock:
            if self._core.loop is None:
                loop = asyncio.new_event_loop()
                thread = threading.Thread(target=loop.run_forever, name="leptoskit-live", daemon=True)
                thread.start()
                self._core.loop = loop
                self._thread = thread
            return self._core.loop

    def _on_loop(self) -> bool:
        return self._thread is not None and threading.get_ident() == self._thread.ident

    def _call(self, function: Callable[..., T], *args: Any) -> T:
        """Runs ``function`` on the connection's loop, and returns its result."""
        if self._core.closed and (self._thread is None or not self._thread.is_alive()):
            # Nothing runs on the loop anymore: the core is the caller's.
            return function(*args)
        loop = self._start()
        if self._on_loop():
            return function(*args)
        done: concurrent.futures.Future[T] = concurrent.futures.Future()

        def run() -> None:
            try:
                done.set_result(function(*args))
            except BaseException as error:  # noqa: BLE001 - handed to the caller
                done.set_exception(error)

        loop.call_soon_threadsafe(run)
        return done.result()


class AsyncLiveConnection:
    """The asynchronous client's live connection: one WebSocket for every watch,
    live mutation and channel, opened on first use on the running event loop
    and kept up until ``aclose()``.

    Read ``status`` or pass a listener to ``on_status`` for a connection banner.
    """

    def __init__(
        self,
        url: str,
        *,
        headers: Optional[Mapping[str, str]] = None,
        options: Optional[LiveOptions] = None,
    ) -> None:
        self.url = url
        self._core = _Core(url, headers or {}, options or LiveOptions())

    @property
    def status(self) -> LiveStatus:
        """Where the connection is now."""
        return self._core.status

    @property
    def version(self) -> int:
        """The last version the connection applied."""
        return self._core.version

    def on_status(self, listener: Callable[[LiveStatus], None]) -> Callable[[], None]:
        """Calls ``listener`` with every status change, until the returned
        function is called."""
        self._core.listeners.append(listener)

        def stop() -> None:
            if listener in self._core.listeners:
                self._core.listeners.remove(listener)

        return stop

    def watch(self, name: str, args: Any) -> AsyncWatch[Any]:
        """Watches the query ``name``."""
        self._bind()
        return AsyncWatch(self._core, name, args)

    async def mutate(self, name: str, args: Any, idempotency_key: Optional[str] = None) -> Any:
        """Runs the mutation ``name`` over the socket, and returns its result once
        this connection's watches show the write (read-your-writes), or after
        ``read_your_writes`` seconds if none of them changes.

        A call made while the connection is down waits for it, and one whose
        answer is lost is sent again with the same request id and key, so it runs
        once.
        """
        self._bind()
        return await self._core.mutate(name, args, idempotency_key)

    def join(
        self,
        name: str,
        key: str,
        args: Any,
        *,
        presence: Any = None,
        on_signal: Optional[Callable[[Any, Optional[str]], None]] = None,
        on_presence: Optional[Callable[[list[Any]], None]] = None,
        on_error: Optional[Callable[[LiveError], None]] = None,
    ) -> Channel[Any, Any]:
        """Joins the instance ``key`` of the channel ``name``."""
        self._bind()
        joined = self._core.join(name, key, args, presence, on_signal, on_presence, on_error)
        return Channel(_direct, self._core, key, joined)

    async def reauthenticate(self) -> None:
        """Sends the token again, such as after signing in."""
        await self._core.reauthenticate()

    async def aclose(self) -> None:
        """Closes the connection for good; pending mutations fail, and watches end."""
        if self._core.loop is None:
            self._core.closed = True
            return
        await self._core.aclose()

    async def __aenter__(self) -> AsyncLiveConnection:
        return self

    async def __aexit__(self, *exc_info: object) -> None:
        await self.aclose()

    def _bind(self) -> None:
        loop = asyncio.get_running_loop()
        if self._core.loop is None:
            self._core.loop = loop
        elif self._core.loop is not loop:
            raise LiveError(0, "the live connection belongs to another event loop")


def _direct(function: Callable[..., T], *args: Any) -> T:
    return function(*args)


def _join_frame(key: str, joined: _Joined) -> dict[str, Any]:
    return {
        "type": "join",
        "channel": key,
        "name": joined.name,
        "args": joined.args,
        "state": joined.state,
    }


async def _write(socket: Any, outbox: asyncio.Queue[str]) -> None:
    while True:
        await socket.send(await outbox.get())


def _abort(socket: Any) -> None:
    transport = getattr(socket, "transport", None)
    if transport is not None:
        transport.abort()


def _connector() -> Any:
    try:
        from websockets.asyncio.client import connect
    except ImportError as error:
        raise ImportError(
            "the live connection needs websockets 13 or newer; install the live extra"
        ) from error
    return connect


def _apply_patch(value: Any, patch: Any) -> Any:
    """Applies a list patch or a page's row and metadata changes together."""
    if not isinstance(patch, dict):
        return value
    field = patch.get("field")
    if isinstance(field, str):
        if not isinstance(value, dict):
            return value
        fields = dict(value)
        changed = patch.get("set")
        if isinstance(changed, dict):
            fields.update(changed)
        if field in fields:
            fields[field] = _apply_rows(fields[field], patch)
        return fields
    return _apply_rows(value, patch)


def _apply_rows(value: Any, patch: dict[str, Any]) -> Any:
    """Removes rows first, then replaces matching ids or inserts new rows."""
    if not isinstance(value, list):
        return value
    remove = patch.get("remove")
    removed = set(remove) if isinstance(remove, list) else set()
    items = [
        item
        for item in value
        if not (isinstance(item, dict) and isinstance(item.get("id"), str) and item["id"] in removed)
    ]
    upserts = patch.get("upsert")
    for upsert in upserts if isinstance(upserts, list) else []:
        if not isinstance(upsert, dict):
            continue
        at = max(0, min(int(_number(upsert.get("at"), len(items))), len(items)))
        row = upsert.get("value")
        row_id = row.get("id") if isinstance(row, dict) else None
        previous = items[at] if at < len(items) else None
        if isinstance(row_id, str) and isinstance(previous, dict) and previous.get("id") == row_id:
            items[at] = row
        else:
            items.insert(at, row)
    return items


def _wire_error(err: Any) -> LiveError:
    error = err if isinstance(err, dict) else {}
    message = error.get("message")
    return LiveError(_number(error.get("status"), 500), str(message if message is not None else "the server failed"))


def _number(value: Any, fallback: Any) -> Any:
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return value
    return fallback


def _present(args: Any) -> Any:
    """The arguments without absent optional fields, as the HTTP calls send them."""
    if args is None:
        return {}
    if isinstance(args, Mapping):
        return {key: item for key, item in args.items() if item is not None}
    return args


def _encode(frame: Any) -> str:
    # Compact, and with characters as they are: what `serde_json` writes.
    return json.dumps(frame, separators=(",", ":"), ensure_ascii=False)


def _resolve(future: asyncio.Future[Any], value: Any) -> None:
    if not future.done():
        future.set_result(value)


def _safely(callback: Callable[..., Any], *args: Any) -> None:
    """Calls an app's callback; a failure in it is logged, not the connection's."""
    try:
        callback(*args)
    except Exception:  # noqa: BLE001 - the app's bug, not the protocol's
        _log.exception("a live callback failed")
