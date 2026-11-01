# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/ifiokjr/leptoskit/releases/tag/v0.1.1) (2026-11-01)

### Features

#### Add scoped process tools, logging and secure randomness

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #62](https://github.com/ifiokjr/leptoskit/pull/62)

Add independent launch-argument, scoped child-process and structured logging crates. Argument snapshots support typed parsing and explicit guest disclosure. Child execution fixes executable, arguments, environment and working directory, bounds output and deadlines, and owns direct-child cancellation/reaping. It requires host configuration and manual asynchronous guest integration.

Logging uses an app-directory factory, bounded JSONL rotation and explicit flush/close. Logging and crypto dispatch on Rust workers. Logging requires Rust 1.89. Extend crypto with operating-system random bytes and UUID v4 while preserving existing digest method IDs; browser randomness uses an explicit feature.
