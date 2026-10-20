---
"release": minor
---

# Add widget testing and device test execution

Add `leptoskit_testing` with semantic finders, event interactions, bounded pumping, viewport changes, and diagnostic tree dumps over the production renderer. Use it for the gallery counter and real AppKit views.

Add `leptoskit test run` for native test executables on desktop, iOS simulators, and Android devices/emulators, plus `leptoskit test web` for browser component tests. Each run retains command output and a JSON report. Devices are selected explicitly, and simulators/emulators started by a run are shut down afterward.
