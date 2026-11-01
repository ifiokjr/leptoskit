//! `#[component]`: a function that `view!` can call with named props.
//!
//! The function keeps its signature, so builder code calls it positionally as
//! before. Beside it the macro declares a type with the same name (types and
//! functions live in separate namespaces) whose `builder()` collects props by
//! name for `view!`:
//!
//! ```text
//! #[component]
//! fn NoteCard(note: &Note, #[prop(optional)] highlight: Option<u32>) -> impl IntoView
//!
//! // declares
//! enum NoteCard {}                     // NoteCard::builder()
//! struct NoteCardBuilder<S0, S1> { … } // .note(…) .highlight(…) .build()
//! ```
//!
//! Each builder slot is `()` until its prop is set and `(T,)` after, so a
//! missing required prop or a prop set twice is a compile error that names
//! the prop. Generics and elided lifetimes move onto the setters that use
//! them and onto `build`, which calls the function.

use std::collections::BTreeSet;

use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use quote::format_ident;
use quote::quote;
use quote::quote_spanned;
use syn::Attribute;
use syn::Expr;
use syn::FnArg;
use syn::GenericParam;
use syn::Generics;
use syn::Ident;
use syn::ItemFn;
use syn::Lifetime;
use syn::LifetimeParam;
use syn::Pat;
use syn::PathArguments;
use syn::ReturnType;
use syn::Type;
use syn::TypeParam;
use syn::TypeParamBound;
use syn::WherePredicate;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit_mut::VisitMut;

/// Expands `#[component]` on a function.
///
/// When the function cannot be a component, the errors are emitted beside
/// the function itself, minus its `#[prop]` attributes, so the rest of the
/// crate still type-checks and rust-analyzer keeps working.
pub fn expand(attribute: &TokenStream, item: TokenStream) -> TokenStream {
	let mut function = match syn::parse2::<ItemFn>(item) {
		Ok(function) => function,
		Err(error) => return error.to_compile_error(),
	};

	let mut errors = Vec::new();

	if !attribute.is_empty() {
		errors.push(syn::Error::new(
			attribute.span(),
			"`#[component]` takes no arguments; configure each prop with `#[prop(…)]`",
		));
	}

	let props = take_props(&mut function, &mut errors);
	check_signature(&function, &mut errors);

	if !errors.is_empty() {
		let errors = errors.iter().map(syn::Error::to_compile_error);

		return quote!(#(#errors)* #function);
	}

	Component::new(function, props).into_tokens()
}

/// One parameter of the component, read from its `#[prop(…)]` attribute.
struct Prop {
	name: Ident,
	/// The parameter's type, with elided lifetimes named and `impl Trait`
	/// replaced by generic parameters, so a builder slot can hold it.
	ty: Type,
	/// The type as written, for the docs.
	written: Type,
	/// Lifetimes and generics introduced for this prop by that rewrite.
	introduced: Vec<GenericParam>,
	docs: Vec<Attribute>,
	kind: PropKind,
	into: bool,
}

enum PropKind {
	Required,
	/// Omitted props take this value; `None` means `Default::default()`.
	Optional {
		default: Option<Box<Expr>>,
		/// Whether the setter takes `T` for an `Option<T>` prop.
		strip: bool,
	},
}

/// Removes `#[prop]` and doc attributes from the parameters and reads them.
fn take_props(function: &mut ItemFn, errors: &mut Vec<syn::Error>) -> Vec<Prop> {
	let mut props = Vec::new();
	let mut rewriter = TypeRewriter::default();

	for input in &mut function.sig.inputs {
		let FnArg::Typed(parameter) = input else {
			errors.push(syn::Error::new(
				input.span(),
				"a component is a free function; `self` cannot be a prop",
			));
			continue;
		};
		let mut docs = Vec::new();
		let mut options = PropOptions::default();
		parameter.attrs.retain(|attribute| {
			if attribute.path().is_ident("doc") {
				docs.push(attribute.clone());
				false
			} else if attribute.path().is_ident("prop") {
				if let Err(error) = options.read(attribute) {
					errors.push(error);
				}

				false
			} else {
				true
			}
		});

		let Pat::Ident(pattern) = parameter.pat.as_ref() else {
			errors.push(syn::Error::new(
				parameter.pat.span(),
				"a prop needs a plain name, like `title: String`, so `view!` can set it",
			));
			continue;
		};

		if pattern.by_ref.is_some() || pattern.subpat.is_some() {
			errors.push(syn::Error::new(
				pattern.span(),
				"a prop needs a plain name, like `title: String`, so `view!` can set it",
			));
			continue;
		}

		let mut ty = parameter.ty.as_ref().clone();
		rewriter.introduced.clear();
		rewriter.visit_type_mut(&mut ty);

		let kind = match options.kind(&ty) {
			Ok(kind) => kind,
			Err(message) => {
				errors.push(syn::Error::new(
					options.span.unwrap_or_else(Span::call_site),
					message,
				));
				continue;
			}
		};

		props.push(Prop {
			name: pattern.ident.clone(),
			written: parameter.ty.as_ref().clone(),
			ty,
			introduced: rewriter.introduced.clone(),
			docs,
			kind,
			into: options.into,
		});
	}

	props
}

#[derive(Default)]
struct PropOptions {
	optional: bool,
	optional_no_strip: bool,
	default: Option<Box<Expr>>,
	into: bool,
	span: Option<Span>,
}

impl PropOptions {
	fn read(&mut self, attribute: &Attribute) -> syn::Result<()> {
		self.span = Some(attribute.span());
		attribute.parse_nested_meta(|meta| {
			if meta.path.is_ident("optional") {
				self.optional = true;
			} else if meta.path.is_ident("optional_no_strip") {
				self.optional_no_strip = true;
			} else if meta.path.is_ident("into") {
				self.into = true;
			} else if meta.path.is_ident("default") {
				self.default = Some(Box::new(meta.value()?.parse()?));
			} else {
				return Err(meta.error(
					"unknown prop option; expected `optional`, `optional_no_strip`, `default = \
					 …`, or `into`",
				));
			}

			Ok(())
		})
	}

	fn kind(&mut self, ty: &Type) -> Result<PropKind, &'static str> {
		let optional_kinds = [
			self.optional,
			self.optional_no_strip,
			self.default.is_some(),
		];

		match optional_kinds.iter().filter(|set| **set).count() {
			0 => Ok(PropKind::Required),
			1 => {
				Ok(PropKind::Optional {
					default: self.default.take(),
					strip: self.optional && option_inner(ty).is_some(),
				})
			}
			_ => {
				Err(
					"choose one of `optional`, `optional_no_strip`, and `default = …`; each makes \
					 the prop optional",
				)
			}
		}
	}
}

/// `T` for a type spelled `Option<T>`.
fn option_inner(ty: &Type) -> Option<&Type> {
	let Type::Path(path) = ty else { return None };
	let segment = path.path.segments.last()?;

	if segment.ident != "Option" {
		return None;
	}

	let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
		return None;
	};

	match arguments.args.first() {
		Some(syn::GenericArgument::Type(inner)) if arguments.args.len() == 1 => Some(inner),
		_ => None,
	}
}

/// Rejects what cannot be a component and normalizes its return type.
fn check_signature(function: &ItemFn, errors: &mut Vec<syn::Error>) {
	let signature = &function.sig;

	if signature.asyncness.is_some() {
		errors.push(syn::Error::new(
			signature.asyncness.span(),
			"a component cannot be `async`; load data outside it and pass the result as a prop",
		));
	}

	if let Some(variadic) = &signature.variadic {
		errors.push(syn::Error::new(
			variadic.span(),
			"a component cannot be variadic",
		));
	}

	if !signature
		.ident
		.to_string()
		.trim_start_matches("r#")
		.starts_with(|first: char| first.is_ascii_uppercase())
	{
		errors.push(syn::Error::new(
			signature.ident.span(),
			"a component's name starts with a capital letter, like `NoteCard`, so `view!` can \
			 tell it from a prop",
		));
	}

	if matches!(signature.output, ReturnType::Default) {
		errors.push(syn::Error::new(
			signature.ident.span(),
			"a component returns its view: add `-> impl IntoView`",
		));
	}
}

/// Names elided lifetimes and turns `impl Trait` into generic parameters, so
/// a parameter's type can be written outside the function. Elision inside
/// `Fn(&T)` sugar and `fn(&T)` pointers is higher-ranked and left alone.
#[derive(Default)]
struct TypeRewriter {
	count: usize,
	introduced: Vec<GenericParam>,
}

impl TypeRewriter {
	fn lifetime(&mut self, span: Span) -> Lifetime {
		self.count += 1;
		let lifetime = Lifetime::new(&format!("'__prop{}", self.count), span);
		self.introduced
			.push(GenericParam::Lifetime(LifetimeParam::new(lifetime.clone())));
		lifetime
	}
}

impl VisitMut for TypeRewriter {
	fn visit_type_mut(&mut self, ty: &mut Type) {
		if let Type::ImplTrait(implementation) = ty {
			let mut bounds = implementation.bounds.clone();

			for bound in &mut bounds {
				self.visit_type_param_bound_mut(bound);
			}

			self.count += 1;
			let ident = format_ident!("__Prop{}", self.count, span = implementation.span());
			self.introduced.push(GenericParam::Type(TypeParam {
				attrs: Vec::new(),
				ident: ident.clone(),
				colon_token: Some(syn::Token![:](implementation.span())),
				bounds,
				default: None,
			}));
			*ty = syn::parse_quote!(#ident);

			return;
		}

		syn::visit_mut::visit_type_mut(self, ty);
	}

	fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
		if reference.lifetime.is_none() {
			reference.lifetime = Some(self.lifetime(reference.and_token.span()));
		}

		syn::visit_mut::visit_type_reference_mut(self, reference);
	}

	fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
		if lifetime.ident == "_" {
			*lifetime = self.lifetime(lifetime.span());
		}
	}

	fn visit_type_fn_ptr_mut(&mut self, _: &mut syn::TypeFnPtr) {}

	fn visit_parenthesized_generic_arguments_mut(
		&mut self,
		_: &mut syn::ParenthesizedGenericArguments,
	) {
	}
}

struct Component {
	function: ItemFn,
	props: Vec<Prop>,
	builder: Ident,
	slots: Vec<Ident>,
}

/// The type a setter takes, and the value it stores: a stripped optional
/// prop takes its inner type, and an `Into` prop takes anything that converts.
fn setter_argument(prop: &Prop) -> (TokenStream, TokenStream) {
	let ty = &prop.ty;

	match (&prop.kind, prop.into) {
		(PropKind::Optional { strip: true, .. }, into) => {
			let inner = option_inner(ty).expect("a stripped prop is an `Option`");

			if into {
				(
					quote!(impl ::core::convert::Into<#inner>),
					quote!(::core::option::Option::Some(value.into())),
				)
			} else {
				(
					inner.to_token_stream(),
					quote!(::core::option::Option::Some(value)),
				)
			}
		}

		(_, true) => {
			(
				quote!(impl ::core::convert::Into<#ty>),
				quote!(value.into()),
			)
		}

		(_, false) => (ty.to_token_stream(), quote!(value)),
	}
}

impl Component {
	fn new(function: ItemFn, props: Vec<Prop>) -> Self {
		let name = &function.sig.ident;
		let builder = format_ident!("{}Builder", name, span = name.span());
		let slots = (0..props.len())
			.map(|index| format_ident!("__S{}", index))
			.collect();
		Self {
			function,
			props,
			builder,
			slots,
		}
	}

	fn into_tokens(mut self) -> TokenStream {
		self.document_props();
		self.qualify_into_view();
		self.enter_scope();
		let function = &self.function;
		let vis = &function.vis;
		let name = &function.sig.ident;
		let builder = &self.builder;
		let slots = &self.slots;

		let fields = self.props.iter().map(|prop| &prop.name).collect::<Vec<_>>();
		let empty = vec![quote!(()); self.props.len()];
		let marker_doc = format!(
			"The `view!` tag for [`{name}`](fn@{name}): `<{name} …/>` sets its props by name \
			 through [`{name}::builder`]."
		);
		let setters = self.setters();
		let build = self.build();
		let requirements = self.requirements();

		quote! {
			#[allow(non_snake_case, clippy::too_many_arguments, clippy::needless_pass_by_value)]
			#function

			#[doc = #marker_doc]
			#[allow(clippy::empty_enums)]
			#vis enum #name {}

			impl #name {
				/// Starts collecting this component's props for `view!`.
				#vis fn builder() -> #builder<#(#empty),*> {
					#builder { #(#fields: ()),* }
				}
			}

			#[doc(hidden)]
			#[must_use = "call `.build()` to render the component"]
			#vis struct #builder<#(#slots),*> {
				#(#fields: #slots),*
			}

			#requirements

			impl<#(#slots),*> #builder<#(#slots),*> {
				#setters
				#build
			}
		}
	}

	/// Opens the component's `DevTools` scope at the top of its body, so the
	/// elements the body creates carry its name. Without the `devtools`
	/// feature the scope is empty and compiles away.
	fn enter_scope(&mut self) {
		let scope = scope_guard();
		let label = self.function.sig.ident.to_string();
		let statement: syn::Stmt = syn::parse_quote! {
			let #scope = ::leptoskit::__private::ComponentScope::enter(#label);
		};
		self.function.block.stmts.insert(0, statement);
	}

	/// Appends the props to the function's docs, as leptos does.
	fn document_props(&mut self) {
		if self.props.is_empty() {
			return;
		}

		let mut lines = vec![String::new(), "# Props".to_owned(), String::new()];

		for prop in &self.props {
			let ty = readable(&prop.written.to_token_stream());

			let requirement = match &prop.kind {
				PropKind::Required => "required".to_owned(),

				PropKind::Optional { default: None, .. } => "optional".to_owned(),
				PropKind::Optional {
					default: Some(default),
					..
				} => format!("default `{}`", readable(&default.to_token_stream())),
			};

			let summary = prop
				.docs
				.iter()
				.filter_map(doc_text)
				.collect::<Vec<_>>()
				.join(" ");
			let summary = summary.trim();

			let separator = if summary.is_empty() { "" } else { ": " };
			lines.push(format!(
				"- **`{}`** (`{ty}`, {requirement}){separator}{summary}",
				prop.name
			));
		}

		for line in lines {
			self.function.attrs.push(syn::parse_quote!(#[doc = #line]));
		}
	}

	/// `impl IntoView` resolves to leptoskit's trait, whatever is imported.
	fn qualify_into_view(&mut self) {
		if let ReturnType::Type(_, output) = &mut self.function.sig.output
			&& let Type::ImplTrait(implementation) = output.as_mut()
		{
			for bound in &mut implementation.bounds {
				if let TypeParamBound::Trait(bound) = bound
					&& bound.path.leading_colon.is_none()
					&& bound.path.segments.len() == 1
					&& bound.path.segments[0].ident == "IntoView"
				{
					let span = bound.path.span();
					bound.path = syn::parse_quote_spanned!(span=> ::leptoskit::IntoView);
				}
			}
		}
	}

	/// The builder type with slot `index` replaced by `slot`.
	fn builder_with(&self, index: usize, slot: &TokenStream) -> TokenStream {
		let builder = &self.builder;
		let slots = self.slots.iter().enumerate().map(|(other, ident)| {
			if other == index {
				slot.clone()
			} else {
				ident.to_token_stream()
			}
		});
		quote!(#builder<#(#slots),*>)
	}

	fn setters(&self) -> TokenStream {
		let setters = (0..self.props.len()).map(|index| self.setter(index));

		quote!(#(#setters)*)
	}

	/// One prop's builder method: it takes the value, fills the prop's slot,
	/// and keeps the other slots as they were.
	fn setter(&self, index: usize) -> TokenStream {
		let builder = &self.builder;
		let generics = &self.function.sig.generics;
		let prop = &self.props[index];
		let name = &prop.name;
		let ty = &prop.ty;
		let slot = &self.slots[index];
		let docs = &prop.docs;
		let (argument, value) = setter_argument(prop);
		let used = used_generics(ty, generics, &prop.introduced);
		let (parameters, predicates) = used.split();
		let next = self.builder_with(index, &quote!((#ty,)));
		let others = self
			.props
			.iter()
			.map(|prop| &prop.name)
			.filter(|field| *field != name)
			.collect::<Vec<_>>();
		// Prefixed, so a prop called `value` cannot shadow the argument.
		let kept = others
			.iter()
			.map(|field| format_ident!("__{}", field.to_string().trim_start_matches("r#")))
			.collect::<Vec<_>>();

		quote_spanned! {name.span()=>
			#(#docs)*
			#[allow(clippy::type_complexity)]
			pub fn #name<#parameters>(self, value: #argument) -> #next
			where
				#slot: ::leptoskit::__private::Unset,
				#predicates
			{
				let Self { #name: _, #(#others: #kept),* } = self;
				#builder { #name: (#value,), #(#others: #kept),* }
			}
		}
	}

	/// Per required prop, a trait that only a filled slot implements, whose
	/// error message names the missing prop.
	fn requirements(&self) -> TokenStream {
		let name = &self.function.sig.ident;
		let vis = &self.function.vis;
		let traits = self.props.iter().filter_map(|prop| {
			if !matches!(prop.kind, PropKind::Required) {
				return None;
			}

			let requirement = requirement_trait(name, &prop.name);
			let message = format!("`<{name}>` is missing the required prop `{}`", prop.name);
			let label = if prop.name == "children" {
				"put its children between `<{name}>` and `</{name}>`"
					.replace("{name}", &name.to_string())
			} else {
				format!("set it with `{}=…`", prop.name)
			};

			Some(quote! {
				#[doc(hidden)]
				#[allow(non_camel_case_types)]
				#[diagnostic::on_unimplemented(message = #message, label = #label)]
				#vis trait #requirement<T> {
					fn value(self) -> T;
				}

				impl<T> #requirement<T> for (T,) {
					fn value(self) -> T {
						self.0
					}
				}
			})
		});
		quote!(#(#traits)*)
	}

	fn build(&self) -> TokenStream {
		let function = &self.function.sig;
		let name = &function.ident;
		let mut parameters = function
			.generics
			.params
			.iter()
			.cloned()
			.map(strip_default)
			.collect::<Vec<_>>();
		parameters.extend(self.props.iter().flat_map(|prop| prop.introduced.clone()));
		let parameters = lifetimes_first(parameters);
		let mut predicates = function
			.generics
			.where_clause
			.as_ref()
			.map(|clause| clause.predicates.iter().cloned().collect::<Vec<_>>())
			.unwrap_or_default();
		let mut values = Vec::new();

		for (prop, slot) in self.props.iter().zip(&self.slots) {
			let field = &prop.name;
			let ty = &prop.ty;

			match &prop.kind {
				PropKind::Required => {
					let requirement = requirement_trait(name, field);
					predicates.push(syn::parse_quote!(#slot: #requirement<#ty>));
					values.push(quote!(#requirement::value(self.#field)));
				}
				PropKind::Optional { default, .. } => {
					let default = default.as_ref().map_or_else(
						|| quote!(::core::default::Default::default()),
						ToTokens::to_token_stream,
					);
					predicates
						.push(syn::parse_quote!(#slot: ::leptoskit::__private::Optional<#ty>));
					values.push(quote! {
						::leptoskit::__private::Optional::value_or(self.#field, || #default)
					});
				}
			}
		}

		let scope = scope_guard();
		let label = self.function.sig.ident.to_string();
		quote! {
			/// Renders the component with the props set so far.
			#[allow(clippy::type_complexity)]
			pub fn build<#(#parameters),*>(self) -> ::leptoskit::__private::Element
			where
				#(#predicates,)*
			{
				// `view!` output can turn into elements only here, after the
				// function returned, so the scope covers this too.
				let #scope = ::leptoskit::__private::ComponentScope::enter(#label);
				::leptoskit::IntoView::into_view(#name(#(#values),*))
			}
		}
	}
}

/// The scope guard's name, unreachable from the component's own code.
fn scope_guard() -> Ident {
	Ident::new("__leptoskit_component_scope", Span::mixed_site())
}

fn requirement_trait(component: &Ident, prop: &Ident) -> Ident {
	let prop = prop.to_string();
	format_ident!("__{}_requires_{}", component, prop.trim_start_matches("r#"))
}

fn strip_default(parameter: GenericParam) -> GenericParam {
	match parameter {
		GenericParam::Type(mut ty) => {
			ty.default = None;
			GenericParam::Type(ty)
		}
		GenericParam::Const(mut constant) => {
			constant.default = None;
			GenericParam::Const(constant)
		}
		lifetime @ GenericParam::Lifetime(_) => lifetime,
	}
}

/// Tokens as a person would write them: `&'a Note<T>`, not `& 'a Note < T >`.
fn readable(tokens: &TokenStream) -> String {
	let mut text = tokens.to_string();
	for (spaced, tight) in [
		(" < ", "<"),
		("< ", "<"),
		(" <", "<"),
		(" >", ">"),
		(" ,", ","),
		(" :: ", "::"),
		(":: ", "::"),
		("& ", "&"),
		(" (", "("),
		("( ", "("),
		(" )", ")"),
		(" . ", "."),
	] {
		text = text.replace(spaced, tight);
	}
	text
}

fn doc_text(attribute: &Attribute) -> Option<String> {
	let syn::Meta::NameValue(meta) = &attribute.meta else {
		return None;
	};
	let Expr::Lit(syn::ExprLit {
		lit: syn::Lit::Str(text),
		..
	}) = &meta.value
	else {
		return None;
	};
	Some(text.value().trim().to_owned())
}

/// The generic parameters and predicates a setter needs: those its prop's
/// type mentions, with bounds that refer only to parameters it declares.
/// Everything else is checked by `build`.
struct UsedGenerics {
	parameters: Vec<GenericParam>,
	predicates: Vec<WherePredicate>,
}

impl UsedGenerics {
	fn split(self) -> (TokenStream, TokenStream) {
		let parameters = self.parameters;
		let predicates = self.predicates;
		(quote!(#(#parameters),*), quote!(#(#predicates,)*))
	}
}

fn used_generics(ty: &Type, generics: &Generics, introduced: &[GenericParam]) -> UsedGenerics {
	let mentioned = names(&quote!(#ty #(#introduced)*));
	let mut parameters = generics
		.params
		.iter()
		.filter(|parameter| mentioned.contains(&parameter_name(parameter)))
		.cloned()
		.map(strip_default)
		.collect::<Vec<_>>();
	parameters.extend(introduced.iter().cloned());
	let declared = parameters
		.iter()
		.map(parameter_name)
		.collect::<BTreeSet<_>>();
	let fn_names = generics
		.params
		.iter()
		.map(parameter_name)
		.collect::<BTreeSet<_>>();
	let only_declared = |tokens: TokenStream| {
		names(&tokens)
			.intersection(&fn_names)
			.all(|name| declared.contains(name))
	};

	for parameter in &mut parameters {
		match parameter {
			GenericParam::Type(ty) => {
				ty.bounds = std::mem::take(&mut ty.bounds)
					.into_iter()
					.filter(|bound| only_declared(bound.to_token_stream()))
					.collect::<Punctuated<_, _>>();
				if ty.bounds.is_empty() {
					ty.colon_token = None;
				}
			}
			GenericParam::Lifetime(lifetime) => {
				lifetime.bounds = std::mem::take(&mut lifetime.bounds)
					.into_iter()
					.filter(|bound| only_declared(bound.to_token_stream()))
					.collect::<Punctuated<_, _>>();
				if lifetime.bounds.is_empty() {
					lifetime.colon_token = None;
				}
			}
			GenericParam::Const(_) => {}
		}
	}

	let predicates = generics
		.where_clause
		.iter()
		.flat_map(|clause| &clause.predicates)
		.filter(|predicate| {
			let tokens = predicate.to_token_stream();
			let mentioned = names(&tokens);
			mentioned.iter().any(|name| declared.contains(name)) && only_declared(tokens)
		})
		.cloned()
		.collect();

	UsedGenerics {
		parameters: lifetimes_first(parameters),
		predicates,
	}
}

/// Rust requires lifetimes before type and const parameters.
fn lifetimes_first(mut parameters: Vec<GenericParam>) -> Vec<GenericParam> {
	parameters.sort_by_key(|parameter| !matches!(parameter, GenericParam::Lifetime(_)));
	parameters
}

fn parameter_name(parameter: &GenericParam) -> String {
	match parameter {
		GenericParam::Lifetime(lifetime) => lifetime.lifetime.to_string(),
		GenericParam::Type(ty) => ty.ident.to_string(),
		GenericParam::Const(constant) => constant.ident.to_string(),
	}
}

/// Every identifier and lifetime in a token stream.
fn names(tokens: &TokenStream) -> BTreeSet<String> {
	let mut names = BTreeSet::new();
	let mut tokens = tokens.clone().into_iter().peekable();

	while let Some(token) = tokens.next() {
		match token {
			TokenTree::Ident(ident) => {
				names.insert(ident.to_string());
			}

			TokenTree::Punct(punct) if punct.as_char() == '\'' => {
				if let Some(TokenTree::Ident(ident)) = tokens.next() {
					names.insert(format!("'{ident}"));
				}
			}

			TokenTree::Group(group) => names.extend(self::names(&group.stream())),
			_ => {}
		}
	}

	names
}

#[cfg(test)]
mod tests {
	use super::*;

	fn pretty(tokens: TokenStream) -> String {
		prettyplease::unparse(&syn::parse2::<syn::File>(tokens).expect("valid Rust"))
	}

	fn errors(attribute: &TokenStream, item: TokenStream) -> Vec<String> {
		let output = expand(attribute, item);
		let file = syn::parse2::<syn::File>(output).expect("valid Rust");
		file.items
			.iter()
			.filter_map(|item| {
				let syn::Item::Macro(item) = item else {
					return None;
				};
				Some(
					item.mac
						.parse_body::<syn::LitStr>()
						.expect("a message")
						.value(),
				)
			})
			.collect()
	}

	#[test]
	fn props_become_a_builder() {
		insta::assert_snapshot!(pretty(expand(
			&TokenStream::new(),
			quote! {
				/// A note in a list.
				pub fn NoteCard<'a, T: Display + 'a>(
					/// The note to show.
					note: &'a Note<T>,
					#[prop(optional)] highlight: Option<u32>,
					#[prop(into, default = "Untitled".into())] title: String,
					#[prop(optional_no_strip)] badge: Option<Element>,
					#[prop(optional, into)] subtitle: Option<String>,
					on_select: impl Fn(&T) + Send + Sync + 'static,
					children: Children,
				) -> impl IntoView
				where
					T: Clone,
				{
					View().children(children)
				}
			}
		)));
	}

	#[test]
	fn components_without_props_still_have_a_builder() {
		insta::assert_snapshot!(pretty(expand(
			&TokenStream::new(),
			quote! {
				fn Divider() -> Element {
					View()
				}
			}
		)));
	}

	#[test]
	fn elided_lifetimes_get_names_outside_higher_ranked_positions() {
		insta::assert_snapshot!(pretty(expand(
			&TokenStream::new(),
			quote! {
				pub(crate) fn Label(
					#[doc(hidden)]
					#[doc = concat!("built")]
					#[allow(unused_mut)]
					text: &str,
					format: fn(&str) -> String,
					cow: Cow<'_, str>,
					mut count: usize,
					items: Vec<impl Into<String>>,
					#[prop(optional)] layout: Layout,
				) -> impl leptoskit::IntoView + use<> {
					format(text)
				}
			}
		)));
	}

	#[test]
	fn generics_used_by_other_props_stay_on_build() {
		let output = expand(
			&TokenStream::new(),
			quote! {
				fn Pair<A: Into<B>, B: Clone, const N: usize>(left: A, right: B, sizes: [u8; N]) -> Element
				where
					A: Clone,
					B: Into<A> + Default,
				{
					View()
				}
			},
		)
		.to_string();
		assert!(
			output.contains("pub fn left < A > (self , value : A)"),
			"{output}"
		);
		assert!(
			output.contains("__S0 : :: leptoskit :: __private :: Unset , A : Clone ,"),
			"{output}"
		);
		assert!(output.contains("pub fn right < B : Clone >"), "{output}");
		assert!(
			output.contains("pub fn sizes < const N : usize >"),
			"{output}"
		);
		assert!(
			output.contains("pub fn build < A : Into < B > , B : Clone , const N : usize >"),
			"{output}"
		);
	}

	#[test]
	fn invalid_components_report_every_problem() {
		assert_eq!(
			errors(
				&quote!(inline),
				quote! {
					async fn note_card(
						self,
						(a, b): (u8, u8),
						ref c: u8,
						#[prop(optional, default = 1)] d: u8,
						#[prop(sometimes)] e: u8,
					) {}
				}
			),
			[
				"`#[component]` takes no arguments; configure each prop with `#[prop(…)]`",
				"a component is a free function; `self` cannot be a prop",
				"a prop needs a plain name, like `title: String`, so `view!` can set it",
				"a prop needs a plain name, like `title: String`, so `view!` can set it",
				"choose one of `optional`, `optional_no_strip`, and `default = …`; each makes the \
				 prop optional",
				"unknown prop option; expected `optional`, `optional_no_strip`, `default = …`, or \
				 `into`",
				"a component cannot be `async`; load data outside it and pass the result as a prop",
				"a component's name starts with a capital letter, like `NoteCard`, so `view!` can \
				 tell it from a prop",
				"a component returns its view: add `-> impl IntoView`",
			]
		);
	}

	#[test]
	fn variadic_and_unparsable_items_are_errors() {
		let output = expand(
			&TokenStream::new(),
			quote!(
				struct NotAFunction;
			),
		)
		.to_string();
		assert!(output.contains("compile_error"), "{output}");
		assert_eq!(
			errors(
				&TokenStream::new(),
				quote!(
					unsafe extern "C" fn Native(count: u8, ...) -> Element {
						View()
					}
				)
			),
			["a component cannot be variadic"]
		);
	}

	#[test]
	fn option_inner_reads_only_single_argument_options() {
		let inner = |ty: Type| option_inner(&ty).map(|inner| inner.to_token_stream().to_string());
		assert_eq!(inner(syn::parse_quote!(Option<u8>)).as_deref(), Some("u8"));
		assert_eq!(
			inner(syn::parse_quote!(std::option::Option<u8>)).as_deref(),
			Some("u8")
		);
		assert_eq!(inner(syn::parse_quote!(Vec<u8>)), None);
		assert_eq!(inner(syn::parse_quote!(Option)), None);
		assert_eq!(inner(syn::parse_quote!(Option<'a>)), None);
		assert_eq!(inner(syn::parse_quote!(&Option<u8>)), None);
	}
}
