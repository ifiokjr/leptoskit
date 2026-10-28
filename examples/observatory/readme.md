# observatory

Observatory, a desktop app for a year of stargazing, and the worked example for `leptoskit`'s `table` and `virtual` features.

- **Catalogue:** a `DataTable` of the forty-four brightest stars. Names sort by their text, and magnitudes and distances by their numbers. A custom filter rule answers `mag<1` and `mag>1.9` alongside word searches, the pages are photographic plates in the observatory's own words, and a hand-built grid ranks the brightest three from the same row pipeline.
- **Sky log:** a year of observations, about twenty thousand rows, under a ribbon of 365 nights with the moon's phase. Both are `VirtualList`s, so only what is on screen exists. Rows with a note are taller, a pressed row opens to show its star and pushes the rest down, the ribbon narrows the log to one night, and recorded observations arrive without disturbing rows already measured.

```bash
cargo run -p observatory_example                          # a window on macOS
cargo run -p observatory_example -- snapshot sky.png      # the window as a PNG, on macOS
cargo run -p observatory_example -- summary               # the accessibility tree, anywhere
cargo test -p observatory_example
```

The tests drive the screen the way a person would, over the real renderer on the headless backend: paging, sorting, and searching the catalogue; scrolling the log and the ribbon; opening rows; and recording live.

The sky is generated from a fixed seed, so every run shows the same year. The window does not scroll the log to a chosen night, because leptoskit cannot set a scroll view's offset yet; picking a night narrows the log instead.
