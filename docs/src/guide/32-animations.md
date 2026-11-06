# Animations

A menu that fades in, a card that springs back after a drag, a spinner: animations make an interface feel responsive. On native platforms your interface runs as a Wasm guest in an interpreter, and computing every frame there would stutter whenever the guest is busy. So leptoskit doesn't do that. You describe an animation once, as part of a widget's properties, and the platform runs it on its own compositor: Core Animation on macOS and iOS, the Android animation framework, and CSS transitions and animations in the browser. This is how React Native Reanimated runs animations on the UI thread instead of in JavaScript.

The quickest way in is Tailwind. This drawer slides and fades into place whenever `open` changes, and its spinner turns until the content arrives:

```rust,ignore
use leptoskit::prelude::*;

#[component]
fn Drawer(open: bool, loading: bool) -> Element {
	view! {
		<View
			class="p-4 rounded-xl bg-white transition duration-300 ease-out"
			class=if open { "opacity-100 translate-y-0" } else { "opacity-0 translate-y-4" }
		>
			{loading.then(|| view! { <View class="w-6 h-6 rounded-full border-2 border-sky-500 animate-spin" /> })}
			<Text>"Recent files"</Text>
		</View>
	}
}
```

The guest sends the new opacity and position once. The platform animates from what is on screen to the new values over 300 ms, even if the guest is busy with other work.

## Tailwind classes

| Classes                                                                                                              | What they do                                                                                                                                  |
| -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `translate-x-*`, `translate-y-*`, `translate-*`, `-translate-*`, `translate-none`                                    | Move the widget's drawing by spacing units, fractions of its own size (`translate-x-1/2`), or arbitrary lengths, without moving its siblings. |
| `scale-*`, `scale-x-*`, `scale-y-*`, `scale-none`                                                                    | Scale by a percentage (`scale-95`) about the widget's center.                                                                                 |
| `rotate-*`, `-rotate-*`, `rotate-[0.25turn]`, `rotate-none`                                                          | Rotate clockwise by degrees, or by turns in brackets.                                                                                         |
| `transition`, `transition-all`, `transition-opacity`, `transition-transform`, `transition-colors`, `transition-none` | Animate changes to those properties instead of jumping. `transition` covers opacity, transforms, and colors.                                  |
| `duration-*`, `delay-*`, `ease-linear`, `ease-in`, `ease-out`, `ease-in-out`, `ease-[cubic-bezier(…)]`               | Time a transition. Without a `duration-*`, a transition takes 150 ms.                                                                         |
| `animate-spin`, `animate-ping`, `animate-pulse`, `animate-bounce`, `animate-none`                                    | Tailwind's looping animations.                                                                                                                |
| `starting:opacity-0`, `starting:scale-95`, …                                                                         | The look a widget enters from when it is added, as CSS `@starting-style`. Needs a `transition` that covers the property.                      |

Transforms change only what is drawn. Layout ignores them, so a moved or scaled widget doesn't push its neighbors.

## Springs, entering, and exiting

For motion Tailwind has no class for, set it on the element. These props take values from `leptoskit::native::animation`:

```rust,ignore
use leptoskit::native::animation::Curve;
use leptoskit::native::animation::Edge;
use leptoskit::native::animation::Presence;
use leptoskit::native::animation::Spring;
use leptoskit::native::animation::Transition;
use leptoskit::prelude::*;

#[component]
fn Toast(message: String) -> Element {
	view! {
		<View
			class="px-4 py-3 rounded-lg bg-slate-900"
			entering=Presence::fade().slide_by(0.0, 16.0).spring(Spring::SNAPPY)
			exiting=Presence::slide(Edge::Right).duration(200.0)
			transition=Transition::spring(Spring::GENTLE)
			layout_transition=Curve::spring(Spring::DEFAULT)
		>
			<Text class="text-white">{message}</Text>
		</View>
	}
}
```

- `transition` animates opacity, transform, and color changes. `Transition::spring` uses a spring and `Transition::timing` uses a duration and easing.
- `entering` animates the widget in when it is added, from the look you describe to its own.
- `exiting` animates it out when it is removed. The platform keeps drawing it until the animation ends, even though your interface no longer has it.
- `layout_transition` animates the widget to its new position and size when layout moves it, such as when a row above it is removed.
- `keyframes` plays a sequence, such as `Keyframes::spin()`. Build your own from `Keyframe::at(offset)` steps with `Keyframes::new`, then `.forever()`, `.repeat(n)`, `.alternate()`, or `.delay(ms)`.
- `transform` sets a `Transform` directly, such as `Transform::rotate(-3.0).scaled(1.05, 1.05)`.

Springs come with presets (`Spring::DEFAULT`, `GENTLE`, `SNAPPY`, `STIFF`, `WOBBLY`). You can also give `Spring::new(stiffness, damping)`, or `Spring::with_duration(ms, bounce)` to describe it as SwiftUI does.

If you know Reanimated, the pieces map like this:

| Reanimated                        | leptoskit                                   |
| --------------------------------- | ------------------------------------------- |
| `withTiming(v, { duration })`     | `Transition::timing(duration, easing)`      |
| `withSpring(v, config)`           | `Transition::spring(Spring::new(..))`       |
| `withRepeat(withSequence(..))`    | `Keyframes::new(..).forever()`              |
| `entering={FadeIn.duration(300)}` | `entering=Presence::fade().duration(300.0)` |
| `exiting={SlideOutLeft}`          | `exiting=Presence::slide(Edge::Left)`       |
| `layout={LinearTransition}`       | `layout_transition=Curve::DEFAULT`          |

Every value is checked before it reaches the platform. Durations and delays are capped at a minute, numbers that aren't finite are dropped, and a sequence keeps at most sixteen keyframes.

## What each platform does

- On **iOS, macOS, and Android**, a widget animating out is drawn above its former siblings until it is gone.
- **iOS and macOS** run everything on Core Animation, with springs as real `CASpringAnimation`s. A transition or an entering or exiting animation that turns a widget more than half a turn takes the shorter way around. Keyframes turn as far as they say. Text color changes don't animate; background and border colors do.
- **Android** runs everything with the Android animation framework, with springs evaluated by the same curve the other platforms use. Only background colors animate; text and border colors jump.
- **Web** turns transforms, transitions, and keyframes into CSS, and entering animations into a CSS animation that runs once the element is shown. Springs become a CSS `linear()` easing sampled from the spring. `exiting` and `layout_transition` don't run in the browser yet: a removed element disappears at once, and a moved one jumps.
- **Windows, Linux, the terminal, and HarmonyOS (experimental)** ignore transforms and animations for now. Values jump to their new state, and a removed widget disappears at once.
