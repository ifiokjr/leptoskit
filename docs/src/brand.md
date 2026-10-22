# Brand

The leptoskit mark is the architecture drawn as three boxes.

The two **ember** boxes form an `L`. They are layout boxes: elements placed by the layout engine and committed as real native widgets. The **mint** box is the last piece of the interface, arriving over the air. It is tilted because it is still landing: code push drops the next version of your interface into a shell that is already installed.

In the wordmark, the dot on the `i` is that same mint piece.

<Logos>
	<Logo src="assets/logo-light.svg" label="logo-light.svg" />
	<Logo src="assets/logo-dark.svg" label="logo-dark.svg" ink />
	<Logo src="assets/mark.svg" label="mark.svg" />
	<Logo src="assets/icon.svg" label="icon.svg" />
</Logos>

## Files

Every file lives in [`docs/src/assets`](https://github.com/ifiokjr/leptoskit/tree/main/docs/src/assets).

| File             | Use it for                                                          |
| ---------------- | ------------------------------------------------------------------- |
| `logo-light.svg` | The full logo on light backgrounds.                                 |
| `logo-dark.svg`  | The full logo on dark backgrounds.                                  |
| `mark.svg`       | The mark alone, on any background, including favicons down to 16px. |
| `icon.svg`       | The mark on an ink tile, for app icons, avatars, and rustdoc.       |
| `social.png`     | The 1280×640 card for link previews and the GitHub social preview.  |

## Colours

<Swatches>
	<Swatch name="Ember warm" color="#ff7a45" />
	<Swatch name="Ember" color="#e5304f" />
	<Swatch name="Mint bright" color="#5ef2c2" />
	<Swatch name="Mint" color="#14bf9b" />
	<Swatch name="Ink" color="#16111f" />
	<Swatch name="Paper" color="#faf6f0" />
</Swatches>

Ember boxes use a gradient from ember warm at the top to ember at the bottom. The mint piece runs from mint bright at the top left to mint at the bottom right.

## Typography

The wordmark is custom monoline lettering drawn as SVG strokes, so it needs no font. The docs set headings in [Bricolage Grotesque](https://fonts.google.com/specimen/Bricolage+Grotesque), body text in [Instrument Sans](https://fonts.google.com/specimen/Instrument+Sans), and code in [JetBrains Mono](https://fonts.google.com/specimen/JetBrains+Mono).

## Using the mark

- Keep the mint piece tilted at −12°. Straightening it removes the one detail that says "over the air".
- Leave clear space of at least the width of the mint piece around the mark.
- Don't recolour the boxes, add outlines, or set the mark on busy imagery. On a coloured background, use `icon.svg`.
