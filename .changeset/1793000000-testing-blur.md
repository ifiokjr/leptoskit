---
"release": minor
---

# Leave text inputs in widget tests with `blur`

`WidgetTester::blur` moves focus out of a text input through its blur handler and pumps once, the way leaving a field does on every backend, so tests can drive validation that runs when the user leaves a field. Like `enter_text`, it rejects controls that are not text inputs.
