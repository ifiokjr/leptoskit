# Running the guest off the UI thread

The generated iOS and Android apps run the Wasm guest on the platform's UI thread, together with everything that turns its output into a transaction: the surface diff, yoga layout, native text measurement, flattening, and encoding. Each of those entry points is a UIKit or Android callback: a display-link tick, a touch, a plugin reply. While the guest works, the UI thread can't scroll, animate, or take a touch.

The arena's `busy` scenario shows the cost. The feed starts the platform's animated scroll, then app logic blocks for 40 ms of every 100 ms. React Native and Lynx run app logic on their own thread, so the scroll keeps moving. In leptoskit the block stops the animation, as it does in Flutter, whose UI and platform threads merged in 3.29.

This note is the design for moving the guest onto a thread of its own on iOS and Android.

## What runs where

| Guest thread                                                                        | UI thread                                                                 |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| The guest: events, display frames, plugin replies, executor polls                   | Creating the app, registering plugins, activating its service, closing it |
| The surface diff, layout, and text measurement                                      | Applying transactions to views                                            |
| Flattening and encoding the binary transaction                                      | The display link, and capturing events                                    |
| Rust plugins installed with `App::install`, answered inline as they are today       | Swift and Kotlin plugins, which are main-actor and main-looper code       |
| Resolving worker and async Rust requests into jobs; polling the application service | Running worker and async jobs on the existing pools                       |
| Rive playback: players are created, advanced, and released here                     | Presenting Rive frames, and deciding which Rive views are visible         |

Text measurement is already safe on another thread. On iOS it uses Core Text with per-thread font caches. On Android the measurer attaches the calling thread to the JVM and builds a new `TextPaint` for each call.

Rive players must stay on the thread that created them (see [rive.md](rive.md)). They are created during rendering, so with this change they are created on the guest thread. They are advanced and released there too.

## One owner at a time

The `App` isn't `Send`: Rive players and plugins installed with `App::install` are not. The guest runtime never shares it. It lends the whole `App` to one thread at a time:

1. The UI thread creates the app, registers plugins, sets the text measurer, enables frames, and activates the service, as it does today.
2. `lk_guest_spawn` lends the `App` to a new thread. From then on the UI thread doesn't touch it.
3. `lk_guest_stop` asks the guest to stop and waits for the step in progress. The guest releases its Rive players on its own thread, then exits. The UI thread owns the `App` again and closes it through the existing `lk_app_begin_close` path.

The thread's start and the join order every access, so the handoff is sound for an `App` that shares nothing with code outside it. One unsafe `Send` wrapper carries the pointer, and its safety comment states that condition.

Waiting in `lk_guest_stop` is bounded by one guest step. A guest stuck in an endless loop would hang the close, but today it would hang the whole UI thread.

## The boundary

A mutex-guarded state with a condition variable carries two queues. The UI side never waits on the guest while holding the mutex.

- **The inbox**, UI to guest:
  - start, resize, events, display frames, plugin replies, worker replies, wakes, Rive ticks, and stop;
  - the first display frame enqueued in place keeps the newest timestamp, so a slow guest never works through a backlog of frames;
  - a scroll event replaces the one still queued for the same view.
- **The outbox**, guest to UI:
  - encoded transactions, in order;
  - native requests, each with its resolved worker or async job when it has one;
  - cancellations and Rive frames;
  - the first failure.
- **Flags:** whether the guest waits for a frame, and whether it owns Rive scenes. The UI side reads these without the mutex.
- **Notification:** when the outbox gains something, the guest calls the platform's notify callback, at most once until the UI thread drains. iOS posts a main-queue block; Android posts to the main looper.

Native requests keep their backpressure. The guest moves at most 64 into the outbox, and refills it when the UI thread takes one. The platform's job budget still bounds work in flight.

After the first failure the guest stops taking commands. The UI side reads the error and closes, as it does now.

## Each step

The guest thread waits until the inbox has something, then takes everything in it and runs the commands in order. Then it does what a pump does today, in the same order:

1. Poll the service.
2. Move cancellations to the outbox.
3. Answer Rust calls inline.
4. Move native requests to the outbox.
5. Render if anything changed.
6. Update the flags, and notify.

## Frames

A display-link tick enqueues the frame and returns. The guest delivers the frame, renders, and posts the transaction. The UI thread applies it on the next main-queue turn, and Core Animation or the Choreographer commits it before the next vsync. If the guest needs more than a frame, the UI thread keeps drawing and scrolling meanwhile. The guest picks up the newest frame next, as `requestAnimationFrame` does in a browser.

The guest and the UI thread can now overlap. The guest builds frame N+1 while the UI thread applies frame N, so a screen that is expensive on both sides draws more often.

## Plugin calls

A Swift or Kotlin plugin call now crosses two threads: guest to UI, then UI to guest. To keep the arena's echo fast, the guest briefly spins before it parks on the condition variable, and the platforms post with their cheapest main-thread hop. Rust plugins answer on the guest thread without touching the UI thread, so they get faster.

This changes one thing for plugin authors. On iOS and Android, a Rust plugin installed with `App::install` runs on the guest thread, not the UI thread. A plugin that needs UIKit or Android views has to be written in Swift or Kotlin, which every bundled mobile plugin already is.

## Rive

A Rive tick becomes a command. It carries the elapsed time, the scale, the flags, and the ids of the Rive views the UI thread can see. The guest advances its players. It copies each rendered frame into an owned buffer for the outbox instead of calling into UIKit. The UI thread presents those frames. Whether another tick is needed comes back as a flag.

This costs one extra copy per frame. A guest step that blocks also delays Rive frames, as it delays everything the guest draws.

## Which apps use it

The generated iOS and Android apps run their guest on this thread. The rest keep the UI-thread runtime:

- **The macOS app:** its Rust plugins (menu, tray, window state, global shortcuts, and haptics) call AppKit and need the main thread.
- **HarmonyOS:** it has its own synchronous host.
- **Tests and tools:** the existing synchronous `lk_app_*` ABI stays for them.

## Alternatives considered

- **Sharing the `App` behind a mutex**, with the UI thread trying the lock for Rive ticks, was rejected for two reasons. The UI thread would still wait on whole guest steps whenever it needed the app. And Rive players would move between threads, which the Rive runtime forbids.
- **Platform queues calling the existing ABI** would mean a `DispatchQueue` on iOS and a `HandlerThread` on Android. This was rejected because it writes the same threading twice, in Swift and Kotlin. It would also need every main-actor type that holds the app pointer to be isolated again: the service registry, the lifecycle, the request executor.
- **A Rust-owned thread**, the chosen design, is written once and tested with cargo. Swift and Kotlin only post work and apply what comes back.

## How it is checked

- **Rust tests** drive a real guest through the thread. They cover:
  - start, events, and display frames;
  - Rust plugin replies inline, and native requests with their backpressure;
  - coalesced frames and scrolls;
  - a failure stopping the guest;
  - stop returning the app with its Rive players released.
- **Swift and Kotlin tests** apply what the guest posts.
- **The iOS controls harness** builds against the new entry points.
- **The arena** re-measures `busy` alongside every other scenario on the simulator and the emulator, so no other metric regresses unseen.
