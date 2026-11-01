---
"release": patch
---

# Device tools no longer wait forever on a pipe a daemon holds

`simctl`, `devicectl`, and `adb` can leave a daemon behind that inherits the tool's output pipes, so reading them to end of file waited for that daemon to exit, which on a wedged machine is never. The tool runner now reads output in chunks, reaps the tool, and stops collecting after a bounded grace period, so a stale device service fails the step or returns the output it produced instead of hanging the run.
