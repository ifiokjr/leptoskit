//! A service that reports what reaches it, so the deploy checks in
//! `.github/workflows/services-deploy.yml` can see what the edge forwards: the
//! cookies it passes on, the cookies it lets the service set, its volume, and
//! the app's private address.
//!
//! The binary is the service; this library is empty.
