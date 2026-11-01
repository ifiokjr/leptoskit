# The live scenario for the generated Python client; see `sdk_live.rs`.
# Runs the scenario on the synchronous `Client`, then a short pass on
# `AsyncClient`, with the generated `sdk_live` package installed.

from __future__ import annotations

import asyncio
import os
import sys
import threading
import traceback
from typing import Any, Callable, Optional

from sdk_live import AsyncClient, Client, LiveOptions, PresenceMember, SdkNote, SdkPresence

server_url = os.environ["SDK_LIVE_URL"]
prefix = os.environ["SDK_LIVE_PREFIX"]
WAIT = 20.0


def timed_out() -> None:
    print("timed out", file=sys.stderr, flush=True)
    os._exit(1)


budget = threading.Timer(60.0, timed_out)
budget.daemon = True
budget.start()


def check(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def say(step: str) -> None:
    print(f"ok {step}", flush=True)


def count(notes: Optional[list[SdkNote]], body: str) -> int:
    return sum(1 for note in notes or [] if note["body"] == body)


def options(token: str) -> LiveOptions:
    return LiveOptions(token=lambda: token, min_backoff=0.05, max_backoff=0.2)


class Changes:
    """Waits for a condition the channel callbacks change, on their thread."""

    def __init__(self) -> None:
        self._changed = threading.Condition()

    def poke(self) -> None:
        with self._changed:
            self._changed.notify_all()

    def until(self, test: Callable[[], bool], what: str) -> None:
        with self._changed:
            if not self._changed.wait_for(test, WAIT):
                raise AssertionError(f"waited too long for {what}")


def names(members: list[PresenceMember[SdkPresence]]) -> str:
    return ",".join(sorted(f"{member['id']}:{member['state']['name']}" for member in members))


def synchronous() -> None:
    ada = Client(server_url, live=options("ada"))
    bob = Client(server_url, live=options("bob"))
    statuses: list[str] = []
    ada.live.on_status(lambda status: statuses.append(status.state))
    ada_notes = ada.sdk_notes.watch(prefix=prefix)
    bob_notes = bob.sdk_notes.watch(prefix=prefix)

    first = ada_notes.wait_for(timeout=WAIT)
    bob_notes.wait_for(timeout=WAIT)
    check(first == [], f"a fresh database has no notes: {first}")
    say("initial")

    # Read-your-writes: when the call returns, Bob's own watch has it.
    note_id = bob.add_sdk_note.live(body=f"{prefix}one")
    check(isinstance(note_id, str), "the mutation returns the id")
    check(count(bob_notes.value, f"{prefix}one") == 1, f"Bob sees his own write: {bob_notes.value}")
    say("read-your-writes")

    # Ada's watch receives it as a patch.
    ada_notes.wait_for(lambda notes: count(notes, f"{prefix}one") == 1, WAIT)
    say("patch")

    # The proxy lets this through, then cuts the connection before the answer.
    dropped = ada.add_sdk_note.live(body=f"{prefix}drop-me")
    check(isinstance(dropped, str), "the re-sent mutation answers")
    ada_notes.wait_for(lambda notes: count(notes, f"{prefix}drop-me") == 1, WAIT)
    check("reconnecting" in statuses, f"Ada reconnected: {statuses}")
    check(count(ada_notes.value, f"{prefix}drop-me") == 1, "the re-sent mutation ran once")
    say("reconnect")

    bob.add_sdk_note.live(body=f"{prefix}two")
    ada_notes.wait_for(lambda notes: count(notes, f"{prefix}two") == 1, WAIT)
    say("resumed")

    # Channels: presence from both, and Bob's signal reaches Ada.
    changes = Changes()
    signals: list[tuple[str, Optional[str]]] = []
    members: dict[str, list[PresenceMember[SdkPresence]]] = {"ada": [], "bob": []}

    def presence_of(who: str) -> Callable[[list[PresenceMember[SdkPresence]]], None]:
        def on_presence(now: list[PresenceMember[SdkPresence]]) -> None:
            members[who] = now
            changes.poke()

        return on_presence

    def on_signal(message: Any, sender: Optional[str]) -> None:
        signals.append((message["text"], sender))
        changes.poke()

    ada_room = ada.sdk_room.join(
        {"room": prefix},
        presence={"name": "Ada"},
        on_presence=presence_of("ada"),
        on_signal=on_signal,
        on_error=lambda error: print(f"ada join: {error.message}", file=sys.stderr),
    )
    bob_room = bob.sdk_room.join({"room": prefix}, presence={"name": "Bob"}, on_presence=presence_of("bob"))
    changes.until(
        lambda: names(members["ada"]) == "ada:Ada,bob:Bob" and names(members["bob"]) == "ada:Ada,bob:Bob",
        "both in the room",
    )
    check(names(ada_room.members) == "ada:Ada,bob:Bob", f"the handle's members: {ada_room.members}")
    bob_room.signal({"text": "hi"})
    changes.until(lambda: len(signals) > 0, "Bob's signal")
    check(signals[0] == ("hi", "bob"), f"the signal: {signals}")
    ada_room.leave()
    bob_room.leave()
    say("channel")

    ada_notes.close()
    bob_notes.close()
    ada.live.close()
    bob.live.close()
    check(ada.live.status.state == "offline", f"closed: {ada.live.status}")


async def asynchronous() -> None:
    async with AsyncClient(server_url, live=options("ada")) as ada:
        async with ada.sdk_notes.watch(prefix=prefix) as notes:
            first = await notes.wait_for(timeout=WAIT)
            check(count(first, f"{prefix}two") == 1, f"the async watch's first value: {first}")
            # Read-your-writes on the caller's event loop.
            await ada.add_sdk_note.live(body=f"{prefix}three")
            check(count(notes.value, f"{prefix}three") == 1, f"the async write: {notes.value}")
            async for value in notes:
                if count(value, f"{prefix}three") == 1:
                    break
    say("async")


try:
    synchronous()
    asyncio.run(asynchronous())
except BaseException:
    traceback.print_exc()
    sys.stdout.flush()
    os._exit(1)

budget.cancel()
