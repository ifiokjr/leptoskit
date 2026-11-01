---
"release": patch
---

# Device commands list devices only when they need to

`leptoskit device screenshot --url …` and the other device commands asked every simulator and Android tool for its devices before running, even when `--url` or `--kind` already said what the target was, and asked twice for some commands. On a Mac whose simulator tools were slow to answer, that held a command for up to four minutes. Only a bare `--device` now lists devices, and the listing asks every tool at once, so a slow tool costs its timeout once.
