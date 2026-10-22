---
"release": patch
---

# Rebuild the served docs when the watcher drops events

Under load, macOS FSEvents and Linux inotify drop events and report only a rescan that names no file. `Site::serve` ignored those, so an edit could fail to rebuild the site or reload open pages. Rescans and watcher errors now count as changes, and the server also compares the book's file sizes and modification times every second, so an edit no watcher reported still rebuilds. The output directory is created before watching so its writes are never mistaken for edits, which could otherwise rebuild in a loop after a failed first build.
