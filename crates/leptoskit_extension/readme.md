# leptoskit_extension

The runtime behind leptoskit's browser extensions, and typed bindings for the WebExtensions APIs that Chromium, Firefox, and Safari share.

An app turns it on with `leptoskit`'s `extension` feature, names its entry points with `leptoskit::extension::entry!`, and builds with `leptoskit build --platform extension`. Every API returns `Error::Unavailable` in a browser that lacks it, rather than throwing, and every event listener is removed when its handle is dropped.
