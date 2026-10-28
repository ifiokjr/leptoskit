# Agent and maintainer notes

Notes for people and coding agents working on leptoskit itself. The public site in `docs/src` is for app authors; anything about how this repository is built, tested, or planned lives here or in [`CONTRIBUTING.md`](../../CONTRIBUTING.md). Start with [`AGENTS.md`](../../AGENTS.md).

| File                                                 | What it covers                                                                                 |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| [docs-site.md](docs-site.md)                         | Who the public docs are for, how to write a page, and what the strict docs build rejects.      |
| [platform-validation.md](platform-validation.md)     | What CI runs per platform, the platform probe, the widget journey, and the native fixtures.    |
| [harmonyos.md](harmonyos.md)                         | The HarmonyOS checks that need no device, the dedicated SDK workflow, and the evidence so far. |
| [plugin-internals.md](plugin-internals.md)           | Plugin bridge ownership, resource limits, copies, and the plugin test suites.                  |
| [plugin-port-inventory.md](plugin-port-inventory.md) | The Expo, Tauri, and Electron port ledger, verification evidence, and completion gates.        |
| [css-coverage.md](css-coverage.md)                   | Which tests back each row of the CSS coverage reference, and how to run them.                  |
| [testing.md](testing.md)                             | What `leptoskit_testing` borrowed from other frameworks, and why.                              |
| [code-splitting.md](code-splitting.md)               | The web and native code-splitting investigation and the proposed shared API.                   |
| [arena-findings.md](arena-findings.md)               | Gaps and slow paths the cross-framework arena found in leptoskit: the performance work list.   |

The Flutter widget audit behind the component catalog's Flutter table is [`.audit/component-catalog/flutter-widgets.tsv`](../../.audit/component-catalog/flutter-widgets.tsv): all 229 entries in the published Flutter index as of 20 October 2026, each with its mapping or missing foundation.
