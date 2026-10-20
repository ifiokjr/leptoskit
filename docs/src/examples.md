# Examples

Every screen below is a leptoskit `Element` tree, laid out by [`leptoskit_yoga`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga) and committed as real AppKit views by [`leptoskit_appkit`](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_appkit). There's no web view and nothing is drawn by hand: text is an `NSTextField`, buttons are AppKit controls, and the images are AppKit's own offscreen render of the view tree at 2x.

The phone screens are rendered on macOS at a 390×844 viewport. They show what the layout engine and the renderer can do, not the iOS or Android backends, which aren't built yet.

<div class="lk-gallery">
	<figure>
		<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/counter.png" alt="A counter screen in the leptoskit palette: a large count of 12, a progress bar at 60%, and a round minus button beside an ember Add one button." loading="lazy" width="390" height="844">
		<figcaption><strong>Counter</strong> A centred card, a progress bar from nested flex boxes, and a pill button that grows to fill the row.</figcaption>
	</figure>
	<figure>
		<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/profile.png" alt="A profile screen with a landscape header, an avatar overlapping it, Message and Follow buttons, a bio, a stats row, and three recent work thumbnails." loading="lazy" width="390" height="844">
		<figcaption><strong>Profile</strong> An avatar positioned over the header, wrapping bio text measured with AppKit's font metrics, and a three-column stats card.</figcaption>
	</figure>
	<figure>
		<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/settings.png" alt="A settings screen with a search field, an account card, and grouped rows with coloured icons, values, chevrons, and switches." loading="lazy" width="390" height="844">
		<figcaption><strong>Settings</strong> Grouped rows with inset separators, trailing values and chevrons, and switches that are AppKit buttons styled as a pill track with a knob.</figcaption>
	</figure>
	<figure>
		<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/photos.png" alt="A photo library with a segmented control and a grid of landscape thumbnails, one spanning two rows and two columns, with video durations overlaid." loading="lazy" width="390" height="844">
		<figcaption><strong>Photos</strong> CSS grid inside a scroll view, with one item spanning two rows and two columns and duration badges positioned over the tiles.</figcaption>
	</figure>
	<figure>
		<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/chat.png" alt="A chat thread with grey incoming bubbles, indigo outgoing bubbles, an image message, a read receipt, and a message composer." loading="lazy" width="390" height="844">
		<figcaption><strong>Chat</strong> Bubbles capped with <code>max_width</code> so text wraps inside them, aligned left or right by <code>align_self</code>.</figcaption>
	</figure>
</div>

<figure class="lk-gallery-wide">
	<img src="https://f002.backblazeb2.com/file/leptoskit/examples/macos/dashboard.png" alt="A desktop store dashboard with a dark sidebar, four stat cards, a twelve-month revenue bar chart, and a list of top products." loading="lazy" width="1200" height="800">
	<figcaption><strong>Dashboard</strong> A 1200×800 desktop layout: a fixed sidebar, a row of stat cards that share the width, a bar chart built from flex items, and a product list.</figcaption>
</figure>

## Run them yourself

The screens live in [`examples/gallery`](https://github.com/ifiokjr/leptoskit/tree/main/examples/gallery). Each one is a plain function returning an `Element`, so the same code is also laid out headlessly in the gallery's tests.

```bash
cargo run -p gallery -- list                  # the screen names
cargo run -p gallery -- window dashboard      # open one in a real macOS window
cargo run -p gallery -- snapshot screenshots  # render every screen to PNG at 2x
```

`window` and `snapshot` need macOS. On other platforms they say so and exit with an error.
