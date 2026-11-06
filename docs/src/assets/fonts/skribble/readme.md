# Skribble lettering

Skribble Noto Sketch Regular and Skribble Mono Playful Regular/Bold come from [openbudgetfun/skribble](https://github.com/openbudgetfun/skribble/tree/00845d0d2a1c07a126079453de342811ee1cff64/packages), commit `00845d0d2a1c07a126079453de342811ee1cff64`.

The original TTF files live in `skribble_font_noto_sketch/assets/fonts` and `skribble_font_recursive/assets/fonts`. These WOFF2 exports were made with fonttools 4.68.0 using `TTFont(source, recalcTimestamp=False)`, setting `flavor = "woff2"`, then saving the result. The original SIL Open Font Licenses are included beside the exports. `LICENSE.skribble` is the upstream MIT license for the drawing system.

`SkribbleNotoSketch-Latin.woff2` and `SkribbleMonoPlayful-Latin.woff2` are the Regular exports cut down to printable ASCII and a little punctuation for the landing page, which preloads them; anything else falls back to the full fonts' fallbacks. They were made with fonttools 4.68.0:

```sh
pyftsubset <Regular>.woff2 --flavor=woff2 --output-file=<Latin>.woff2 \
	--unicodes="U+0020-007E,U+00A0,U+00B7,U+2013-2014,U+2018-201D,U+2022,U+2026,U+2190-2199" \
	--layout-features='kern,liga,calt'
```

The sketch font has no arrows, so only the mono subset carries them. Keep the `unicode-range` rules in `crates/leptoskit_app/web/landing.css` in step with that list.
