---
"leptoskit_plugin_arguments": minor
"leptoskit_plugin_shell": minor
"leptoskit_plugin_logging": minor
"leptoskit_plugin_crypto": minor
---

# Add scoped process tools, logging and secure randomness

Add independent launch-argument, scoped child-process and structured logging crates. Argument snapshots support typed parsing and explicit guest disclosure. Child execution fixes executable, arguments, environment and working directory, bounds output and deadlines, and owns direct-child cancellation/reaping. It requires host configuration and manual asynchronous guest integration.

Logging uses an app-directory factory, bounded JSONL rotation and explicit flush/close. Logging and crypto dispatch on Rust workers. Logging requires Rust 1.89. Extend crypto with operating-system random bytes and UUID v4 while preserving existing digest method IDs; browser randomness uses an explicit feature.
