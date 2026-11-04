---
leptoskit_testing: major
---

# leptoskit_testing: native widget tests

`leptoskit_testing` runs Flutter-style widget tests over the production renderer and Yoga layout, so a screen can be tested without a device or simulator.

`WidgetTester` renders a screen, finds elements by text, label, role, or test id with Testing Library-style queries (`get_by_text`, `query_by_label`, and the rest), acts on them, and pumps the renderer a bounded number of times while waiting for a result. The `native-ui` feature goes further and drives the real app through each platform's own automation: XCUITest on iOS, UI Automator on Android, XCTest on macOS, UI Automation on Windows, and AT-SPI on Linux.
