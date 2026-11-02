---
leptoskit_uikit: breaking
---

# leptoskit_uikit: the iOS backend

`leptoskit_uikit` is the iOS backend: it renders shared leptoskit primitives as real `UIView` and `UIViewController` content, with UIKit text measurement and native controls.

It consumes the same Yoga mutation stream as every other backend. The generated iOS app hosts it through `UIKitHost`; the backend crate draws a wider component set than the Swift renderer used by the generated app.
