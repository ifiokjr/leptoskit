//! Markup and component macros for leptoskit's cross-platform components.
//!
//! Use these through `leptoskit::prelude`, which also exports the traits the
//! expansions name. See the `leptoskit` crate for the guide.
//!
//! - [`view!`](macro@view) parses [rstml](https://github.com/rs-tml/rstml)
//!   markup and expands it into the same builder calls you would write by hand,
//!   so it renders on the web and on every native backend.
//! - [`#[component]`](macro@component) gives a function a props builder, so
//!   `view!` can call it with named props, while builder code keeps calling it
//!   positionally.

mod component;
mod platform_module;
mod view;

/// Builds a view from markup.
///
/// Every tag is a component: a built-in such as `<View>` or `<Text>`, or a
/// function marked `#[component]`.
///
/// - `name=value` calls the builder method `name`, so `class="p-4"` applies
///   Tailwind and `<Image source=url/>` sets a built-in's prop. A bare `name`
///   passes `true`, and a tuple spreads into several arguments: `border=(1.0,
///   color)`. Wrap a tuple in braces to pass it whole.
/// - `on:press=handler` calls `on_press`; any `on:event` calls `on_event`.
/// - Children are text in quotes, `{expressions}`, and nested tags. `Text` and
///   `Button` take text children as their content.
/// - `<>…</>`, or more than one root, makes a `Fragment`.
///
/// One root tag evaluates to an `Element`.
#[proc_macro]
pub fn view(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
	view::expand(tokens.into()).into()
}

/// Makes a function a component that `view!` can call with named props.
///
/// The function keeps its signature. Its name must start with a capital
/// letter, and it returns `impl IntoView`, an `Element`, or anything else
/// that implements `IntoView`. Each parameter is a prop:
///
/// - `#[prop(optional)]` may be omitted, defaulting to `Default::default()`; an
///   `Option<T>` prop then takes a plain `T`.
/// - `#[prop(optional_no_strip)]` may be omitted and takes the `Option` itself.
/// - `#[prop(default = expr)]` may be omitted, defaulting to `expr`.
/// - `#[prop(into)]` accepts anything that converts into the prop's type.
/// - A `children: Children` prop receives the tag's children.
///
/// Doc comments on parameters are listed in the component's docs.
#[proc_macro_attribute]
pub fn component(
	attribute: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	component::expand(&attribute.into(), item.into()).into()
}

/// Declares a module with an optional platform-specific source file.
///
/// ```ignore
/// leptoskit::platform_module!(pub mod profile = "routes/profile");
/// ```
///
/// Paths are relative to the application's `src` directory, including in
/// nested modules. `routes/profile.rs` is required as the shared fallback;
/// `routes/profile.ios.rs` replaces it for the `ios` application feature,
/// and likewise for `web`, `android`, `macos`, `windows`, and `linux`.
/// Imports keep using `profile` on every platform. Ordinary Rust `mod`
/// declarations remain available for modules that never need an override.
///
/// Selected modules use Rust's `#[path]` lookup rules: inside
/// `src/routes/profile.rs` or `profile.ios.rs`, a plain `mod details;`
/// searches `src/routes/details.rs`, rather than
/// `src/routes/profile/details.rs`. Register children with their
/// source-root-relative stem to retain the logical directory and allow
/// independent platform overrides:
///
/// ```ignore
/// // Inside src/routes/profile.rs and any profile.<platform>.rs override:
/// leptoskit::platform_module!(pub mod details = "routes/profile/details");
/// ```
///
/// The selected file remains a normal module: `self`, `super`, inner
/// attributes, and compiler source locations retain their usual meaning.
///
/// The application must call `leptoskit_build::platforms()` in its build
/// script so source additions and removals invalidate Cargo's cache and
/// native Wasm guests receive the selected device identity.
#[proc_macro]
pub fn platform_module(tokens: proc_macro::TokenStream) -> proc_macro::TokenStream {
	platform_module::expand(tokens.into()).into()
}
