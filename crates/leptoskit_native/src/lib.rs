//! The leptoskit native renderer.
//!
//! A React Native-style renderer, fully rewritten in Rust: apps describe their
//! interface as a tree of [`Element`]s, and the renderer turns successive
//! descriptions into minimal mutation batches that a platform
//! [`NativeBackend`] applies to real native widgets — never a web view.
//!
//! The pipeline per commit:
//!
//! 1. [`NativeSurface::set_root`] receives the new element tree.
//! 2. The surface diffs it against the committed shadow tree and produces a
//!    [`MutationList`] (create, update props, insert, remove, delete).
//! 3. The backend applies the batch to real widgets.
//! 4. [`leptoskit_yoga`] lays the tree out, and only changed frames are pushed
//!    as [`Mutation::UpdateLayout`].
//! 5. Events flow back through [`NativeSurface::dispatch_event`], bubbling
//!    through handlers exactly like the DOM.
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/icon.svg"
)]
#![doc(
	html_favicon_url = "https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg"
)]

pub mod backend;
pub mod components;
pub mod controls;
pub mod element;
pub mod props;
pub mod surface;
pub mod tailwind;
pub mod view;

#[cfg(feature = "web")]
pub mod web;

pub use backend::HeadlessBackend;
pub use backend::Mutation;
pub use backend::MutationList;
pub use backend::NativeBackend;
pub use controls::ControlTreeError;
pub use controls::ControlValueError;
pub use controls::Progress;
pub use controls::ProgressValue;
pub use controls::RadioGroup;
pub use controls::SliderState;
pub use element::Element;
pub use element::NativeEventSlot;
pub use props::ApproxTextMeasurer;
pub use props::Color;
pub use props::EventOutcome;
pub use props::FontWeight;
pub use props::NativeEvent;
pub use props::NativeKind;
pub use props::NativeProps;
pub use props::PropValue;
pub use props::SharedTextMeasurer;
pub use props::TextAlign;
pub use props::TextMeasurer;
pub use props::TextStyle;
pub use surface::Frame;
pub use surface::NativeSurface;
pub use tailwind::ClassError;
pub use tailwind::StyleContext;
pub use tailwind::Tailwind;
pub use view::Children;
pub use view::Fragment;
pub use view::IntoChildren;
pub use view::IntoView;
