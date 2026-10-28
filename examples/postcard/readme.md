# postcard

Postcard, a card home from a trip in five languages, and the worked example for typed translations.

The messages live in `locales/*.arb`, one file per language, and `leptoskit l10n` writes `src/l10n.rs` from them: one method per message, typed by its placeholders. English is the template; Polish counts with four plural forms, Arabic with six and right to left, Japanese with one, and Brazilian Portuguese is half translated, so its missing messages show the English.

The card has a plural for the days away and the photos taken, a select for the weather and for who signs it, and a plural inside a select for the letters waiting at home. Stamps switch the language, sliders change the counts, and tapping the weather or the signature moves their select on. Beside the card, a gauge shows the CLDR plural category every language picks for the same number of days: drag the slider and watch Polish move from one to few to many while Japanese never leaves other.

```bash
cargo run -p postcard_example                          # a window on macOS, in your LANG
cargo run -p postcard_example -- snapshot card.png ar  # the Arabic window as a PNG, on macOS
cargo run -p postcard_example -- summary pl            # the Polish accessibility tree, anywhere
cargo test -p postcard_example
```

After editing an ARB file, run `leptoskit l10n` here to rewrite `src/l10n.rs`, or `leptoskit l10n --check` to fail when someone forgot; `leptoskit dev` and `leptoskit build` regenerate it too. The tests check the committed module matches the ARB files.

Direction does not pass from a parent element to its children yet, so the Arabic screen sets right to left on every element of the tree (`screen::right_to_left`).
