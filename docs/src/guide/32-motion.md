# Motion

Movement tells people what changed and where to look: a card that rises as it scrolls into view, a list whose items fall in one after another, a count that pops when it goes up. On the web that usually means hand-written keyframes, a scroll listener, and a check for people who ask for less motion, kept in step with the markup by hand.

In leptoskit, motion is part of the element. You describe the poses an element moves between and what drives the movement, and the renderer writes the rest:

```rust,ignore
use std::time::Duration;

use leptoskit::prelude::*;

#[component]
pub fn WhatsNew() -> Element {
	let rise = Motion::enter(Pose::new().y(24.0).opacity(0.0)).spring(Spring::GENTLE);

	view! {
		<View class="gap-3 p-6" stagger={Duration::from_millis(80)}>
			<Heading aria_level=2 motion={rise.clone()}>"What's new"</Heading>
			<Text motion={rise.clone()}>"Drafts save while you're offline."</Text>
			<Text motion={rise}>"Sync finishes twice as fast."</Text>
		</View>
	}
}
```

The heading and both lines start 24 points low and transparent, and spring into place as the page loads, 80 milliseconds apart.

On the web this becomes CSS animations with no script. On macOS, iOS, Android, Windows, Linux, the terminal, and HarmonyOS (experimental), the elements show at rest: motion is a web feature today, and native apps never receive it. See [what each platform does](#what-each-platform-does).

## Poses

A `Pose` is how an element looks at one point of a motion. It can set:

| Method    | What it changes                                         |
| --------- | ------------------------------------------------------- |
| `opacity` | Transparency, from `0.0` to `1.0`.                      |
| `x`, `y`  | The offset from where layout put the element.           |
| `scale`   | The size, where `1.0` is the size layout gave it.       |
| `rotate`  | The rotation, in degrees clockwise.                     |
| `clip`    | How much is clipped away from each edge, as an `Inset`. |
| `blur`    | A blur radius.                                          |

These are the properties that can change without moving anything else, so a motion never shifts the content around it. Each value a pose sets stands in for the element's own value while the motion runs, and a property no pose sets never animates. `Pose::REST` is an element at rest: opaque, in place, full size, upright, unclipped, and sharp.

`Inset` describes a clip as fractions of the element's size. `Inset::from_left(0.0)` shows nothing, revealed from the left edge; `Inset::NONE` shows everything. Moving between the two draws an element in like a pen stroke.

## What drives a motion

**Appearing.** `Motion::enter(pose)` moves from `pose` to the element's own style once, when it first appears. Set its timing with `duration`, `delay`, and `easing`, or hand the timing to a spring with `spring`.

**Scrolling into view.** `in_view(range)` ties the motion to the element's own passage through the viewport instead of time, so it runs forwards as the element scrolls in and backwards as it scrolls out:

```rust,ignore
let arrive = Motion::enter(Pose::new().y(48.0).rotate(-2.0)).in_view(ViewRange::entry());

view! { <View class="rounded-xl p-6" motion={arrive}>"Pinned notes"</View> }
```

`ViewRange::entry()` is the time between the element's top edge entering the viewport and the whole element being inside it. `ViewRange::exit()`, `cover()`, and `contain()` cover the other phases, and `ViewRange::during(phase, from, to)` picks part of one.

**Following a container.** For a sequence that plays out while one large section scrolls past, name the section with `timeline`, and have any element inside it `follow` that name:

```rust,ignore
let fade_through = Motion::keyframes([
	(0.0, Pose::new().opacity(0.0)),
	(0.3, Pose::REST),
	(0.7, Pose::REST),
	(1.0, Pose::new().opacity(0.0)),
])
.easing(Easing::Linear)
.follow("tour", ViewRange::contain());

view! {
	<View class="gap-24 py-24" timeline="tour">
		<Text class="text-4xl" motion={fade_through}>"Every screen"</Text>
	</View>
}
```

`Motion::keyframes` takes any number of poses at points from `0.0` to `1.0`. Points outside that range are clamped, and poses with values that aren't finite numbers are dropped.

## Timing and springs

`Easing` sets how a motion moves between poses: `Linear`, `Out` (the default: quick, then settling), `In`, `InOut`, a cubic `Bezier`, or `Steps` for a frame-by-frame look. A motion that sets no duration takes 600 milliseconds.

A `Spring` moves like a weight on a spring instead of following a curve. Choose `Spring::GENTLE`, `Spring::SNAPPY`, or `Spring::BOUNCY`, or describe one with `Spring::new(stiffness, damping, mass)`. A spring sets its own duration, the time it takes to settle, unless you set one.

To play an enter motion again, give it a new `key`. A counter can pass its count, so every press pops the number once:

```rust,ignore
let pop = Motion::enter(Pose::new().scale(1.35)).spring(Spring::BOUNCY).key(count);
```

## Staggering

`stagger(step)` on a container delays each child's motion by its position times `step`. A child's own delay adds to its place in the stagger, and children without motion are unaffected.

## People who ask for less motion

Every motion respects the system's reduce-motion setting. When it is on, elements appear at rest at once, scrolled sections stay still, and content never waits for an animation.

## What each platform does

- **Web:** motion renders as CSS animations. Springs become `linear()` easing curves. Scroll-driven motion uses CSS scroll timelines; browsers without them, such as Firefox and Safari releases before scroll timelines shipped, show the elements at rest. Pages without JavaScript animate too, because no script is involved.
- **macOS, iOS, Android, Windows, Linux, the terminal, and HarmonyOS (experimental):** elements show their resting pose. Native apps never receive motion, so adding it doesn't change what they build or how they lay out. Use [Rive animations](20-rive.md) for motion that has to play inside native apps today.
