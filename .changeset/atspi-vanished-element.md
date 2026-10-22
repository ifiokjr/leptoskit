---
"release": patch
---

# See through Linux accessibles destroyed mid-observation

Native waits on Linux now treat the AT-SPI driver's `'NoneType' object is not iterable` error as an element that vanished while being read, the same way they already treated `stale element reference`. selenium-webdriver-at-spi raises it when an accessible is destroyed during its tree walk, as a portal dialog's are while the dialog closes, so `expect_absent` and `expect_visible` keep polling until their deadline instead of failing. Actions such as `tap` still fail on it.
