---
leptoskit_plugin_shell: major
---

# leptoskit_plugin_shell: child processes

`leptoskit_plugin_shell` is the leptoskit plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities, such as running a bundled tool with `--version`.

Add `leptoskit_plugin_shell` as a direct dependency, select the `shell` plugin in `native.plugins`, and declare each command under `[native.plugin-options.shell.commands]`: an absolute executable and working directory, the allowed argument slots, an explicit environment, output limits, and a deadline. Guests enable the `guest` feature and call `guest::ShellClient::execute` with a command name and arguments; they never send a shell command string. Commands run on macOS, Linux, and Windows, and cancellation, timeouts, and shutdown clean up the child's process group or Windows Job Object. The generated Windows and Linux apps don't register runtime plugins yet, so there a host constructs `Shell` itself; mobile and the browser have no process backend.
