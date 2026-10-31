# Agent and maintainer notes

Notes for people and coding agents working on leptoskit itself. The public site in `docs/src` is for app authors; anything about how this repository is built, tested, or planned lives here or in [`CONTRIBUTING.md`](../../CONTRIBUTING.md). Start with [`AGENTS.md`](../../AGENTS.md).

| File                                                 | What it covers                                                                                                                                                          |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [docs-site.md](docs-site.md)                         | Who the public docs are for, how to write a page, and what the strict docs build rejects.                                                                               |
| [platform-validation.md](platform-validation.md)     | What CI runs per platform, the platform probe, the widget journey, and the native fixtures.                                                                             |
| [harmonyos.md](harmonyos.md)                         | The HarmonyOS checks that need no device, the dedicated SDK workflow, and the evidence so far.                                                                          |
| [tui.md](tui.md)                                     | The terminal backend's cell model, input ownership, runners, and coverage floor.                                                                                        |
| [plugin-internals.md](plugin-internals.md)           | Plugin bridge ownership, resource limits, copies, and the plugin test suites.                                                                                           |
| [rive.md](rive.md)                                   | Native Rive ownership, raster limits, image formats, frame scheduling, and verification.                                                                                |
| [plugin-port-inventory.md](plugin-port-inventory.md) | The Expo, Tauri, and Electron port ledger, verification evidence, and completion gates.                                                                                 |
| [css-coverage.md](css-coverage.md)                   | Which tests back each row of the CSS coverage reference, and how to run them.                                                                                           |
| [testing.md](testing.md)                             | What `leptoskit_testing` borrowed from other frameworks, and why.                                                                                                       |
| [code-splitting.md](code-splitting.md)               | The web and native code-splitting investigation and the proposed shared API.                                                                                            |
| [arena-findings.md](arena-findings.md)               | Gaps and slow paths the cross-framework arena found in leptoskit: the performance work list.                                                                            |
| [preview-sandbox.md](preview-sandbox.md)             | The proposed preview sandbox: real environments for every platform on one page, and examples.                                                                           |
| [database-plan.md](database-plan.md)                 | The approved plan for `leptoskit_db`: schema as Rust structs, generated migrations, policies, shapes, geospatial and vectors.                                           |
| [auth-plan.md](auth-plan.md)                         | The approved plan for `leptoskit_auth` and `leptoskit_idp`: passkeys, MFA, third-party login, account merging, and acting as an identity provider.                      |
| [idp-certification.md](idp-certification.md)         | What OpenID certification of `leptoskit_idp` needs: the profiles that fit, what blocks them, and the order of work.                                                     |
| [auth-threat-model.md](auth-threat-model.md)         | What sign-in protects, from whom, the control and test for each threat, and the known gaps.                                                                             |
| [plugin-platforms-plan.md](plugin-platforms-plan.md) | The proposed plan for running every runtime plugin on every platform: Windows, Linux and HarmonyOS hosts, per-platform metadata, and ports.                             |
| [extension-plan.md](extension-plan.md)               | The approved plan for browser extensions as a platform: Chromium, Firefox, and Safari on macOS and iOS, driven by the Commonplace example.                              |
| [devtools-plan.md](devtools-plan.md)                 | The approved plan for the leptoskit debugger: one hub for every platform, the hub page, the DevTools panel, and MCP tools.                                              |
| [publishing-plan.md](publishing-plan.md)             | The approved plan for `leptoskit publish` and `leptoskit skill`: the extension stores, App Store Connect, notarized macOS apps, Google Play, and agent-driven releases. |

The Flutter widget audit behind the component catalog's Flutter table is [`.audit/component-catalog/flutter-widgets.tsv`](../../.audit/component-catalog/flutter-widgets.tsv): all 229 entries in the published Flutter index as of 20 October 2026, each with its mapping or missing foundation.
