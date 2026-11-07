# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.1) (2026-11-07)

### Changed

- **No package-specific changes were recorded; `leptoskit_auth` was updated to 0.3.1 as part of group `release`.**

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_auth: sign-in and sessions

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_auth` is sign-in for leptoskit apps, stored in the app's own database: the `auth` schema pack, revocable sessions, email codes, authenticator apps with recovery codes and step-up, passkeys, third-party providers, opt-in passwords, Solana wallets, account merging, and the signed-in user that access policies read.

Enable `auth` on `leptoskit`, plus `auth-passkeys`, `auth-providers`, `auth-passwords`, `auth-solana`, `auth-smtp`, or `auth-webhook` in the server's crate for the parts that need them. The headless client works on the web and in the generated iOS, Android, and macOS apps, where it uses the passkeys, web-auth, secure-store, and HTTP plugins. `leptoskit_idp` turns the same accounts into an OIDC provider.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
