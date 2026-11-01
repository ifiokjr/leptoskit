//! Markdown with leptos components, in the style of MDX, and the static
//! documentation site generator built on it.
//!
//! A document is GitHub-flavoured Markdown where a capitalised tag is a leptos
//! component:
//!
//! ```markdown
//! # Installing
//!
//! <Callout kind="tip">
//!
//! Install the CLI once and every project can use it.
//!
//! </Callout>
//! ```
//!
//! [`Document::parse`] reads the source, [`render`] turns it into a leptos
//! [`AnyView`](leptos::prelude::AnyView) with the [`Components`] you register,
//! and [`Site`] builds a whole book of documents, laid out by an mdBook-style
//! `SUMMARY.md`, into static HTML with navigation, search, and checked links.
//!
//! ```
//! use leptoskit_docs::Components;
//!
//! let html = leptoskit_docs::render_to_html(
//! 	"<Callout kind=\"tip\">\n\nRun `lk doctor` first.\n\n</Callout>",
//! 	&Components::new(),
//! )
//! .unwrap();
//!
//! assert!(html.starts_with("<aside data-kind=\"tip\" class=\"lk-callout\">"));
//! ```

mod components;
mod diagnostic;
mod document;
mod highlight;
mod layout;
mod props;
mod render;
mod serve;

mod site;
mod summary;

pub use components::Component;
pub use components::Components;
pub use diagnostic::Diagnostic;
pub use diagnostic::Error;
pub use document::Document;
pub use props::PropError;
pub use props::Props;
pub use render::Heading;
pub use render::Link;
pub use render::Rendered;
pub use render::Renderer;
pub use render::Section;
pub use render::render;
pub use render::render_to_html;
pub use serve::Server;
pub use site::Build;
pub use site::Site;
pub use summary::Chapter;
pub use summary::Summary;
pub use summary::SummaryItem;
