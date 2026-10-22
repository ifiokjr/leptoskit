# Built-in components

<Callout kind="warning" title="Mind the gap">

Callouts hold **Markdown**, including lists:

- one
- two

</Callout>

<Callout>A note by default, written inline.</Callout>

<Steps>

### Install the CLI

Run `cargo install leptoskit_cli`.

### Create a project

Run `lk new my_app`.

</Steps>

<Cards>
	<Card title="Read the guide" href="guide/install.md#setup">
		Start from nothing.
	</Card>
	<Card title="No link">
		A card without a target.
	</Card>
</Cards>

<Tabs>
	<Tab label="macOS">

	```bash
	brew install thing
	```

	</Tab>
	<Tab label="Linux">

	```bash
	apt install thing
	```

	</Tab>
</Tabs>

<!-- Comments are dropped. -->

<figure class="shot">
	<img src="assets/shot.png" alt="A screenshot" width="390" />
	<figcaption>Plain <strong>HTML</strong> elements pass through.<br />With line breaks.</figcaption>
</figure>

<details open>
	<summary>Details</summary>

	Hidden *Markdown*.

</details>

<>A fragment renders only its children.</>

Inline <kbd>⌘K</kbd> elements and braces {stay} literal.
