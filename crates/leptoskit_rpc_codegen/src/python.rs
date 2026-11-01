//! The Python client: a package with `TypedDict` models and standard-library
//! transport, plus an `httpx`-backed async client.
//!
//! Models are `TypedDict`s rather than classes so values stay plain `dict`s
//! that round-trip through `json` unchanged — the generated code never has to
//! guess how to serialize a user-defined class. Type aliases are written as
//! quoted `TypeAlias` strings, which type checkers understand and which free
//! the module from declaration-order constraints.
//!
//! Queries, mutations and channels also run over the live socket: the
//! runtime in `live.py` speaks the protocol, and the client gives each query
//! a `.watch(...)`, each mutation a `.live(...)`, and each channel a
//! `.join(...)` beside the one-shot calls.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashMap;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::PY_RESERVED_METHODS;
use crate::naming::PY_RESERVED_PARAMETERS;
use crate::naming::PY_RESERVED_TYPES;
use crate::naming::Scope;
use crate::naming::component_names;
use crate::naming::py_identifier;
use crate::naming::py_package_name;
use crate::naming::py_string;
use crate::naming::type_name;
use crate::schema::ObjectShape;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiChannel;
use crate::spec::ApiFunction;
use crate::spec::ApiKind;
use crate::spec::ApiSpec;
use crate::spec::HttpInput;

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let module = py_package_name(&context.options.package_name);
	let mut types = PyTypes::new(context);
	let (methods, channels) = member_names(context);
	let models = models_module(context, &mut types);
	let client = client_module(context, &mut types, &methods, &channels);
	let mut live = context.banner("#");
	live.push_str(include_str!("templates/live.py"));
	Generated {
		files: vec![
			GeneratedFile::scaffold("pyproject.toml", pyproject(context, &module)),
			GeneratedFile::code(
				format!("{module}/__init__.py"),
				init_module(context, &types),
			),
			GeneratedFile::code(format!("{module}/models.py"), models),
			GeneratedFile::code(format!("{module}/client.py"), client),
			GeneratedFile::code(format!("{module}/live.py"), live),
			GeneratedFile::scaffold("README.md", readme(context, &module, &methods, &channels)),
		],
	}
}

/// Tracks the model names in declaration order, plus synthesized `TypedDict`s
/// for anonymous object schemas.
struct PyTypes {
	/// Every public model name, for `__all__`.
	names: Vec<String>,
	scope: Scope,
	/// The bound name of every component schema, by schema name.
	components: HashMap<String, String>,
	/// Synthesized `TypedDict` definitions, emitted before the aliases.
	synthesized: Vec<String>,
}

impl PyTypes {
	/// A table with every component's name bound up front.
	fn new(context: &Context<'_>) -> Self {
		let mut scope = Scope::new(PY_RESERVED_TYPES);
		let components = component_names(context.spec, &mut scope);
		let names = context
			.spec
			.schemas
			.keys()
			.map(|name| components[name].clone())
			.collect();
		Self {
			names,
			scope,
			components,
			synthesized: Vec::new(),
		}
	}

	fn reserve(&mut self, wanted: &str) -> String {
		let name = self.scope.bind(wanted);
		self.names.push(name.clone());
		name
	}

	/// A Python type expression for `schema`, synthesizing a `TypedDict` named
	/// after `hint` when it is an anonymous object.
	fn render(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> String {
		let analyzed = analyze(schema);

		let base = match analyzed.shape {
			Shape::Ref(name) => {
				self.components
					.get(name)
					.cloned()
					.unwrap_or_else(|| "Any".to_owned())
			}
			Shape::Null => "None".to_owned(),
			Shape::Boolean => "bool".to_owned(),
			Shape::Integer { .. } => "int".to_owned(),
			Shape::Number { .. } => "float".to_owned(),
			Shape::String {
				values: Some(values),
				..
			} => {
				format!(
					"Literal[{}]",
					values
						.iter()
						.map(|value| py_string(value))
						.collect::<Vec<_>>()
						.join(", ")
				)
			}
			Shape::String {
				constant: Some(constant),
				..
			} => format!("Literal[{}]", py_string(constant)),
			Shape::String { .. } => "str".to_owned(),
			Shape::Array { items } => {
				format!(
					"list[{}]",
					items.map_or_else(
						|| "Any".to_owned(),
						|items| self.render(items, &format!("{hint}Item"), spec)
					)
				)
			}
			Shape::Tuple(items) => {
				let parts: Vec<String> = items
					.iter()
					.enumerate()
					.map(|(index, item)| self.render(item, &format!("{hint}{index}"), spec))
					.collect();
				format!("tuple[{}]", parts.join(", "))
			}
			Shape::Object(object) => {
				if object.properties.is_empty() {
					match object.additional {
						Some(values) => {
							format!(
								"dict[str, {}]",
								self.render(values, &format!("{hint}Value"), spec)
							)
						}
						None => "dict[str, Any]".to_owned(),
					}
				} else {
					let name = self.reserve(&type_name(hint));
					let definition = typed_dict(&name, None, &object, self, spec, &name);
					self.synthesized.push(definition);
					name
				}
			}
			Shape::Map { values } => {
				format!(
					"dict[str, {}]",
					values.map_or_else(
						|| "Any".to_owned(),
						|values| self.render(values, &format!("{hint}Value"), spec)
					)
				)
			}
			Shape::Union(variants) => {
				let parts: Vec<String> = variants
					.iter()
					.enumerate()
					.map(|(index, variant)| {
						let variant_hint = variant_hint(variant).map_or_else(
							|| format!("{hint}Variant{}", index + 1),
							|tag| format!("{hint}{}", type_name(tag)),
						);
						self.render(variant, &variant_hint, spec)
					})
					.collect();
				format!("Union[{}]", parts.join(", "))
			}
			Shape::Any => "Any".to_owned(),
		};

		if analyzed.nullable && base != "None" {
			format!("Optional[{base}]")
		} else {
			base
		}
	}
}

/// A discriminator value to name an enum variant's synthesized type after.
fn variant_hint(variant: &Value) -> Option<&str> {
	let object = variant.as_object()?;
	let properties = object.get("properties")?.as_object()?;
	properties.values().find_map(|property| {
		property
			.as_object()
			.and_then(|property| property.get("const"))
			.and_then(Value::as_str)
	})
}

/// A `TypedDict` definition. Class syntax when every key is a plain
/// identifier; functional syntax otherwise.
fn typed_dict(
	name: &str,
	docs: Option<&str>,
	object: &ObjectShape<'_>,
	types: &mut PyTypes,
	spec: &ApiSpec,
	hint: &str,
) -> String {
	let fields = typed_fields(object, types, spec, hint);
	let mut out = if fields.iter().all(|(key, ..)| py_is_plain_key(key)) {
		class_typed_dict(name, docs, &fields)
	} else {
		functional_typed_dict(name, docs, &fields)
	};

	out.push('\n');
	out
}

/// A field's key, type, whether it's required, and its docs.
type Field = (String, String, bool, Option<String>);

/// Each property's field, in the order the object lists them.
fn typed_fields(
	object: &ObjectShape<'_>,
	types: &mut PyTypes,
	spec: &ApiSpec,
	hint: &str,
) -> Vec<Field> {
	let mut fields = Vec::new();

	for (property, schema) in &object.properties {
		let required = object.is_required(property) && !has_default(schema);
		let rendered = types.render(schema, &format!("{hint}{}", type_name(property)), spec);
		fields.push((
			(*property).to_owned(),
			rendered,
			required,
			description(schema).map(ToOwned::to_owned),
		));
	}

	fields
}

/// `class Name(TypedDict, total=False)`, for keys that are identifiers.
fn class_typed_dict(name: &str, docs: Option<&str>, fields: &[Field]) -> String {
	let mut out = format!("class {name}(TypedDict, total=False):\n");

	if let Some(docs) = docs {
		out.push_str(&docstring(docs, "\t"));
	}

	if fields.is_empty() {
		out.push_str("\tpass\n");
	}

	for (key, rendered, required, field_docs) in fields {
		write_field_docs(&mut out, field_docs.as_deref(), "\t");
		let annotation = if *required {
			format!("Required[{rendered}]")
		} else {
			format!("NotRequired[{rendered}]")
		};

		out.push_str(&format!("\t{key}: {annotation}\n"));
	}

	out
}

/// `Name = TypedDict(...)`, for keys that aren't identifiers.
fn functional_typed_dict(name: &str, docs: Option<&str>, fields: &[Field]) -> String {
	let mut out = format!("{name} = TypedDict(\n\t{},\n\t{{\n", py_string(name));

	for (key, rendered, required, field_docs) in fields {
		write_field_docs(&mut out, field_docs.as_deref(), "\t\t");
		let annotation = if *required {
			format!("Required[{}]", py_string(rendered))
		} else {
			format!("NotRequired[{}]", py_string(rendered))
		};

		out.push_str(&format!("\t\t{}: {annotation},\n", py_string(key)));
	}

	out.push_str("\t},\n\ttotal=False,\n)\n");

	if let Some(docs) = docs {
		out.push_str(&format!("{name}.__doc__ = {}\n", py_string(docs)));
	}

	out
}

/// A field's docs, one comment line each.
fn write_field_docs(out: &mut String, docs: Option<&str>, indent: &str) {
	let Some(docs) = docs else {
		return;
	};

	for line in docs.lines() {
		out.push_str(&format!("{indent}# {line}\n"));
	}
}

/// Whether a wire name can be a `class`-syntax `TypedDict` key.
fn py_is_plain_key(name: &str) -> bool {
	let mut characters = name.chars();
	characters
		.next()
		.is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
		&& characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn docstring(docs: &str, indent: &str) -> String {
	let safe = docs.replace("\"\"\"", "\\\"\\\"\\\"");
	let mut lines = safe.lines();
	let first = lines.next().unwrap_or_default();
	let rest: Vec<&str> = lines.collect();

	if rest.is_empty() {
		return format!("{indent}\"\"\"{first}\"\"\"\n");
	}

	let mut out = format!("{indent}\"\"\"{first}\n");

	for line in rest {
		if line.is_empty() {
			out.push('\n');
		} else {
			out.push_str(&format!("{indent}{line}\n"));
		}
	}

	out.push_str(&format!("{indent}\"\"\"\n"));
	out
}

fn models_module(context: &Context<'_>, types: &mut PyTypes) -> String {
	let mut definitions = String::new();
	let mut aliases = String::new();

	for (schema_name, schema) in &context.spec.schemas {
		let name = types.components[schema_name].clone();
		let analyzed = analyze(schema);

		match (&analyzed.shape, analyzed.nullable) {
			(Shape::Object(object), false) if !object.properties.is_empty() => {
				definitions.push_str(&typed_dict(
					&name,
					description(schema),
					object,
					types,
					context.spec,
					&name,
				));
			}

			_ => {
				let rendered = types.render(schema, &name, context.spec);
				if let Some(docs) = description(schema) {
					for line in docs.lines() {
						aliases.push_str(&format!("# {line}\n"));
					}
				}

				aliases.push_str(&format!("{name}: TypeAlias = {}\n\n", py_string(&rendered)));
			}
		}
	}

	let mut out = format!(
		"{}{docs}\n\nfrom __future__ import annotations\n\nfrom typing import Any, Literal, \
		 NotRequired, Optional, Required, TypeAlias, TypedDict, Union\n\n",
		context.banner("#"),
		docs = py_string(&format!(
			"Types for the {} {} RPC.",
			context.spec.title, context.spec.version
		))
	);
	// Synthesized dicts come first: class bodies only hold lazy annotations,
	// so order among them does not matter, while aliases are quoted.
	for synthesized in &types.synthesized {
		out.push_str(synthesized);
	}

	out.push_str(&definitions);
	out.push_str(&aliases);
	out
}

/// The keyword-only parameters of a method, with wire-name mapping.
struct Signature {
	parameters: String,
	payload: String,
}

fn signature(function: &ApiFunction, types: &mut PyTypes, spec: &ApiSpec) -> Signature {
	if function.params.is_empty() {
		return Signature {
			parameters: String::new(),
			payload: "{}".to_owned(),
		};
	}

	let mut parameters = vec!["*".to_owned()];
	let mut payload = Vec::new();
	let hint = type_name(&function.name);

	for (param, identifier) in function.params.iter().zip(parameter_names(function)) {
		let rendered = types.render(
			&param.schema,
			&format!("{hint}{}", type_name(&param.name)),
			spec,
		);

		if param.is_required() {
			parameters.push(format!("{identifier}: {rendered}"));
		} else {
			let rendered = if rendered.starts_with("Optional[") {
				rendered
			} else {
				format!("Optional[{rendered}]")
			};

			parameters.push(format!("{identifier}: {rendered} = None"));
		}

		payload.push(format!("{}: {identifier}", py_string(&param.name)));
	}

	Signature {
		parameters: parameters.join(", "),
		payload: format!("{{{}}}", payload.join(", ")),
	}
}

/// The member name of every callable function and then every channel, in
/// order, clear of the clients' own attributes and of each other.
fn member_names(context: &Context<'_>) -> (Vec<String>, Vec<String>) {
	let mut scope = Scope::new(PY_RESERVED_METHODS);
	let methods = context
		.functions()
		.map(|function| scope.bind(&py_identifier(&function.name)))
		.collect();
	let channels = context
		.spec
		.channels
		.iter()
		.map(|channel| scope.bind(&py_identifier(&channel.name)))
		.collect();
	(methods, channels)
}

/// The keyword parameter of every argument, in order.
fn parameter_names(function: &ApiFunction) -> Vec<String> {
	let mut scope = Scope::new(PY_RESERVED_PARAMETERS);
	function
		.params
		.iter()
		.map(|param| scope.bind(&py_identifier(&param.name)))
		.collect()
}

/// A function's docstring text: its docs, each documented argument, and
/// whether it is deprecated.
fn function_docs(function: &ApiFunction) -> String {
	let mut docs = function.docs.clone();
	let documented: Vec<(&crate::spec::ApiParam, String)> = function
		.params
		.iter()
		.zip(parameter_names(function))
		.filter(|(param, _)| !param.docs.is_empty())
		.collect();

	if !documented.is_empty() {
		docs.push_str("\n\nArguments:");

		for (param, identifier) in documented {
			docs.push_str(&format!("\n\t{identifier}: {}", param.docs.trim()));
		}
	}

	if function.deprecated {
		docs.push_str("\n\nDeprecated.");
	}

	docs
}

/// `docs` as a docstring at `indent`, or nothing when there are none.
fn optional_docstring(docs: &str, indent: &str) -> String {
	if docs.is_empty() {
		String::new()
	} else {
		docstring(docs, indent)
	}
}

/// `docs` followed by a paragraph saying how to use a live member.
fn with_usage(docs: &str, usage: &str) -> String {
	if docs.is_empty() {
		usage.to_owned()
	} else {
		format!("{docs}\n\n{usage}")
	}
}

/// What one operation's members render from.
struct Operation<'a> {
	/// The member's name on the clients.
	member: &'a str,
	/// The operation's wire name.
	name: String,
	/// The keyword parameters, with the idempotency key for a mutation.
	parameters: String,
	/// The one-shot call's arguments to `_call`.
	call: String,
	/// The live call's payload and idempotency key.
	payload: String,
	key: &'static str,
	output: String,
	docs: String,
}

impl Operation<'_> {
	fn separator(&self) -> &'static str {
		if self.parameters.is_empty() { "" } else { ", " }
	}
}

/// A query or mutation: a property on each client returning a helper that
/// is callable for the one-shot HTTP call, with the live form beside it,
/// `.watch(...)` for a query and `.live(...)` for a mutation.
fn live_operation(
	operation: &Operation<'_>,
	kind: ApiKind,
	helpers: &mut Scope,
	out: &mut LiveCode,
) {
	let Operation {
		member,
		name,
		parameters,
		call,
		payload,
		key,
		output,
		docs,
	} = operation;
	let separator = operation.separator();
	let base = type_name(member);
	let (suffix, usage) = if kind == ApiKind::Query {
		(
			"Query",
			"Call it for one result, or ``.watch(...)`` it for every change.",
		)
	} else {
		(
			"Mutation",
			"Call it over HTTP, or ``.live(...)`` over the live connection.",
		)
	};
	let sync = helpers.bind(&format!("_{base}{suffix}"));
	let asynchronous = helpers.bind(&format!("_Async{base}{suffix}"));
	let property_docs = docstring(&with_usage(docs, usage), "\t\t");
	let call_docs = optional_docstring(docs, "\t\t");
	out.sync.push_str(&format!(
		"\n\t@property\n\tdef {member}(self) -> {sync}:\n{property_docs}\t\treturn {sync}(self)\n"
	));
	out.asynchronous.push_str(&format!(
		"\n\t@property\n\tdef {member}(self) -> {asynchronous}:\n{property_docs}\t\treturn \
		 {asynchronous}(self)\n"
	));

	let (sync_live, async_live) = if kind == ApiKind::Query {
		let watch = |watch: &str, verb: &str| {
			format!(
				"\n\tdef watch(self{separator}{parameters}) -> \
				 {watch}[{output}]:\n\t\t\"\"\"Watches the result: {verb} the first value and \
				 each new one, until\n\t\t``close()``. A failure is raised as a ``LiveError``, \
				 and the watch recovers on the\n\t\tnext change.\n\t\t\"\"\"\n\t\treturn \
				 self._client.live.watch({name}, {payload})\n"
			)
		};
		(
			watch("Watch", "iterate for"),
			watch("AsyncWatch", "``async for``"),
		)
	} else {
		let live = |prefix: &str, wait: &str| {
			format!(
				"\n\t{prefix}def live(self{separator}{parameters}) -> {output}:\n\t\t\"\"\"Runs \
				 the mutation over the live connection. It returns once this \
				 client's\n\t\twatches show the write, and a call whose answer is lost is sent \
				 again after a\n\t\treconnect with the same key, so it runs \
				 once.\n\t\t\"\"\"\n\t\treturn {wait}self._client.live.mutate({name}, \
				 {payload}{key})\n"
			)
		};
		(live("", ""), live("async ", "await "))
	};

	out.helpers.push_str(&format!(
		"\n\nclass {sync}:\n\tdef __init__(self, client: Client) -> None:\n\t\tself._client = \
		 client\n\n\tdef __call__(self{separator}{parameters}) -> \
		 {output}:\n{call_docs}\t\treturn self._client._call({call})\n{sync_live}"
	));
	out.helpers.push_str(&format!(
		"\n\nclass {asynchronous}:\n\tdef __init__(self, client: AsyncClient) -> \
		 None:\n\t\tself._client = client\n\n\tasync def __call__(self{separator}{parameters}) -> \
		 {output}:\n{call_docs}\t\treturn await self._client._call({call})\n{async_live}"
	));
}

/// A channel: a property on each client returning a helper whose
/// `.join(args, ...)` returns the joined channel.
fn channel_member(
	channel: &ApiChannel,
	member: &str,
	types: &mut PyTypes,
	spec: &ApiSpec,
	helpers: &mut Scope,
	out: &mut LiveCode,
) {
	let hint = type_name(&channel.name);
	let args = types.render(&channel.args_schema, &format!("{hint}Args"), spec);
	let message = types.render(&channel.message_schema, &format!("{hint}Message"), spec);
	let presence = types.render(&channel.presence_schema, &format!("{hint}Presence"), spec);
	let optional_presence = if presence.starts_with("Optional[") {
		presence.clone()
	} else {
		format!("Optional[{presence}]")
	};
	let fields = channel
		.fields
		.iter()
		.map(|field| py_string(field))
		.collect::<Vec<_>>()
		.join(", ");
	let mut docs = channel.docs.clone();

	if channel.deprecated {
		docs.push_str(if docs.is_empty() {
			"Deprecated."
		} else {
			"\n\nDeprecated."
		});
	}

	let helper = helpers.bind(&format!("_{}Channel", type_name(member)));
	let property_docs = docstring(
		&with_usage(&docs, "``.join(...)`` it for signals and presence."),
		"\t\t",
	);

	for client in [&mut out.sync, &mut out.asynchronous] {
		client.push_str(&format!(
			"\n\t@property\n\tdef {member}(self) -> {helper}:\n{property_docs}\t\treturn \
			 {helper}(self)\n"
		));
	}

	out.helpers.push_str(&format!(
		"\n\nclass {helper}:\n\tdef __init__(self, client: Client | AsyncClient) -> \
		 None:\n\t\tself._client = client\n\n\tdef join(\n\t\tself,\n\t\targs: \
		 {args},\n\t\t*,\n\t\tpresence: {optional_presence} = None,\n\t\ton_signal: \
		 Optional[Callable[[{message}, Optional[str]], None]] = None,\n\t\ton_presence: \
		 Optional[Callable[[list[PresenceMember[{presence}]]], None]] = None,\n\t\ton_error: \
		 Optional[Callable[[LiveError], None]] = None,\n\t) -> Channel[{message}, \
		 {presence}]:\n\t\t\"\"\"Joins the channel: ``on_signal`` gets each signal another member \
		 sends, and\n\t\twho sent it, and ``on_presence`` everyone in the channel after each join \
		 and\n\t\tleave. With ``Client`` they run on the live connection's thread. The \
		 join\n\t\tis made again after a reconnect, until ``leave()``.\n\t\t\"\"\"\n\t\treturn \
		 self._client.live.join(\n\t\t\t{name},\n\t\t\tchannel_key([{fields}], args),\n",
		name = py_string(&channel.name),
	));
	out.helpers.push_str(JOIN_OPTIONS);
}

/// The end of every channel's `join`: the options, passed on as they came.
const JOIN_OPTIONS: &str = r"			args,
			presence=presence,
			on_signal=on_signal,
			on_presence=on_presence,
			on_error=on_error,
		)
";

/// The live members of each client, and the helper classes they return.
#[derive(Default)]
struct LiveCode {
	sync: String,
	asynchronous: String,
	helpers: String,
}

fn client_module(
	context: &Context<'_>,
	types: &mut PyTypes,
	names: &[String],
	channels: &[String],
) -> String {
	let mut methods = String::new();
	let mut async_methods = String::new();
	let mut live = LiveCode::default();
	// The helper classes are private, so they only need to clear each other.
	let mut helpers = Scope::new(&[]);

	for (function, name) in context.functions().zip(names) {
		let Signature {
			parameters,
			payload,
		} = signature(function, types, context.spec);
		let output = types.render(
			&function.output_schema,
			&format!("{}Output", type_name(&function.name)),
			context.spec,
		);

		let encoding = match function.input {
			HttpInput::Query => "query",
			HttpInput::Form => "form",
			HttpInput::Json | HttpInput::Other { .. } => "json",
		};
		let docs = function_docs(function);

		// A mutation takes an idempotency key, so a retried call runs once.
		let (parameters, key) = if function.kind.is_idempotent_with_key() {
			let keyword = "idempotency_key: Optional[str] = None";
			let parameters = if parameters.is_empty() {
				format!("*, {keyword}")
			} else {
				format!("{parameters}, {keyword}")
			};

			(parameters, ", idempotency_key")
		} else {
			(parameters, "")
		};
		let call = format!(
			"{}, {}, {}, {payload}{key}",
			py_string(&function.method),
			py_string(&function.path),
			py_string(encoding),
		);

		if matches!(function.kind, ApiKind::Query | ApiKind::Mutation) {
			let operation = Operation {
				member: name,
				name: py_string(&function.name),
				parameters,
				call,
				payload,
				key,
				output,
				docs,
			};
			live_operation(&operation, function.kind, &mut helpers, &mut live);
			// In spec order, with the plain methods.
			methods.push_str(&std::mem::take(&mut live.sync));
			async_methods.push_str(&std::mem::take(&mut live.asynchronous));
			continue;
		}

		let docs = optional_docstring(&docs, "\t\t");
		let separator = if parameters.is_empty() { "" } else { ", " };
		methods.push_str(&format!(
			"\n\tdef {name}(self{separator}{parameters}) -> {output}:\n{docs}\t\treturn \
			 self._call({call})\n"
		));
		async_methods.push_str(&format!(
			"\n\tasync def {name}(self{separator}{parameters}) -> {output}:\n{docs}\t\treturn \
			 await self._call({call})\n"
		));
	}

	for (channel, member) in context.spec.channels.iter().zip(channels) {
		channel_member(
			channel,
			member,
			types,
			context.spec,
			&mut helpers,
			&mut live,
		);
	}

	methods.push_str(&live.sync);
	async_methods.push_str(&live.asynchronous);
	let imports = if types.names.is_empty() {
		String::new()
	} else {
		format!("from .models import {}\n", types.names.join(", "))
	};

	let banner = context.banner("#");
	let docs = py_string(&format!(
		"A typed client for the {} {} RPC.",
		context.spec.title, context.spec.version
	));
	let base_url = py_string(&context.server_url.clone().unwrap_or_default());
	let live_path = py_string(&context.spec.live_path);

	// Pieces rather than one template: rustfmt reflows long escaped literals.
	[
		banner.as_str(),
		&docs,
		CLIENT_IMPORTS,
		&imports,
		"\nDEFAULT_SERVER_URL = ",
		&base_url,
		SERVER_URL_DOCS,
		&live_path,
		LIVE_PATH_DOCS,
		RUNTIME,
		"\n",
		SYNC_CLIENT,
		&methods,
		"\n\n",
		ASYNC_CLIENT,
		&async_methods,
		&live.helpers,
	]
	.concat()
}

/// What `client.py` imports, after its docstring.
const CLIENT_IMPORTS: &str = r"

from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Mapping, Optional

from .live import AsyncLiveConnection, AsyncWatch, Channel, LiveConnection, LiveError, LiveOptions, PresenceMember, Watch, channel_key, live_url
";

/// `DEFAULT_SERVER_URL`'s docs, up to `LIVE_PATH`'s value.
const SERVER_URL_DOCS: &str = r#"
"""The server origin baked in at generation time; empty means same origin."""

LIVE_PATH = "#;

/// `LIVE_PATH`'s docs.
const LIVE_PATH_DOCS: &str = r#"
"""Where the server serves the live socket, relative to its origin."""

"#;

/// The synchronous client, up to its generated methods.
const SYNC_CLIENT: &str = r#"class Client(_BaseClient):
	"""A synchronous client over the standard library.

	Pass ``server_url`` to point it at a server, ``headers`` for authorization, and
	``timeout`` in seconds. ``live`` sets how the live connection signs in and
	reconnects (install the ``live`` extra).
	"""

	@property
	def live(self) -> LiveConnection:
		"""The live connection: one WebSocket for every watch, live mutation and
		channel, opened on first use on a background thread. Read ``live.status``
		or pass a listener to ``live.on_status`` for a connection banner, and call
		``live.close()`` when done.
		"""
		if self._live is None:
			self._live = LiveConnection(
				live_url(self.base_url, LIVE_PATH),
				headers=self.headers,
				options=self.live_options,
			)
		return self._live

	def _call(self, method: str, path: str, encoding: str, payload: Mapping[str, Any], idempotency_key: Optional[str] = None) -> Any:
		url, headers, data = self._prepare(method, path, encoding, payload, idempotency_key)
		request = urllib.request.Request(url, data=data, method=method, headers=headers)
		try:
			with urllib.request.urlopen(request, timeout=self.timeout) as response:
				status = response.status
				body = response.read().decode("utf-8")
		except urllib.error.HTTPError as error:
			body = error.read().decode("utf-8", "replace")
			raise FnError.from_response(error.code, body) from error
		except urllib.error.URLError as error:
			raise FnError("network", f"could not reach {url}: {error.reason}") from error
		return _decode(status, body)
"#;

/// The asynchronous client, up to its generated methods.
const ASYNC_CLIENT: &str = r#"class AsyncClient(_BaseClient):
	"""An asynchronous client over ``httpx`` (install the ``async`` extra).

	Pass an existing ``httpx.AsyncClient`` as ``http`` to share connections.
	"""

	def __init__(
		self,
		base_url: str = DEFAULT_SERVER_URL,
		*,
		headers: Optional[Mapping[str, str]] = None,
		timeout: float = 30.0,
		http: Any = None,
		live: Optional[LiveOptions] = None,
	) -> None:
		super().__init__(base_url, headers=headers, timeout=timeout, live=live)
		if http is None:
			try:
				import httpx
			except ImportError as error:  # pragma: no cover
				raise ImportError(
					"AsyncClient needs httpx; install the async extra or pass http="
				) from error
			http = httpx.AsyncClient(timeout=timeout)
		self.http = http

	@property
	def live(self) -> AsyncLiveConnection:
		"""The live connection: one WebSocket for every watch, live mutation and
		channel, opened on first use on the running event loop. Read
		``live.status`` or pass a listener to ``live.on_status`` for a connection
		banner; ``aclose()`` closes it.
		"""
		if self._live is None:
			self._live = AsyncLiveConnection(
				live_url(self.base_url, LIVE_PATH),
				headers=self.headers,
				options=self.live_options,
			)
		return self._live

	async def aclose(self) -> None:
		"""Closes the live connection, if it opened, and the underlying ``httpx`` client."""
		if self._live is not None:
			await self._live.aclose()
		await self.http.aclose()

	async def __aenter__(self) -> "AsyncClient":
		return self

	async def __aexit__(self, *exc_info: object) -> None:
		await self.aclose()

	async def _call(self, method: str, path: str, encoding: str, payload: Mapping[str, Any], idempotency_key: Optional[str] = None) -> Any:
		url, headers, data = self._prepare(method, path, encoding, payload, idempotency_key)
		try:
			response = await self.http.request(method, url, headers=headers, content=data)
		except Exception as error:  # httpx.TransportError and friends
			raise FnError("network", f"could not reach {url}: {error}") from error
		if response.status_code >= 400:
			raise FnError.from_response(response.status_code, response.text)
		return _decode(response.status_code, response.text)
"#;

fn init_module(context: &Context<'_>, types: &PyTypes) -> String {
	let mut exports: Vec<String> = vec![
		"AsyncClient".to_owned(),
		"Client".to_owned(),
		"DEFAULT_SERVER_URL".to_owned(),
		"FnError".to_owned(),
		"to_query_string".to_owned(),
	];
	exports.extend(
		[
			"AsyncLiveConnection",
			"AsyncWatch",
			"Channel",
			"LIVE_PATH",
			"LiveConnection",
			"LiveError",
			"LiveOptions",
			"LiveStatus",
			"PresenceMember",
			"Watch",
			"channel_key",
		]
		.map(ToOwned::to_owned),
	);
	exports.extend(types.names.iter().cloned());
	let quoted: Vec<String> = exports.iter().map(|name| py_string(name)).collect();
	let model_imports = if types.names.is_empty() {
		String::new()
	} else {
		format!("from .models import {}\n", types.names.join(", "))
	};

	format!(
		"{}{docs}\n\nfrom .client import AsyncClient, Client, DEFAULT_SERVER_URL, FnError, \
		 LIVE_PATH, to_query_string\nfrom .live import AsyncLiveConnection, AsyncWatch, Channel, \
		 LiveConnection, LiveError, LiveOptions, LiveStatus, PresenceMember, Watch, \
		 channel_key\n{model_imports}\n__all__ = [{}]\n",
		context.banner("#"),
		quoted.join(", "),
		docs = py_string(&format!(
			"A typed client for the {} {} RPC.",
			context.spec.title, context.spec.version
		))
	)
}

fn pyproject(context: &Context<'_>, module: &str) -> String {
	format!(
		"{banner}[build-system]\nrequires = [\"hatchling\"]\nbuild-backend = \
		 \"hatchling.build\"\n\n[project]\nname = {name}\nversion = {version}\ndescription = \
		 {description}\nreadme = \"README.md\"\nrequires-python = \">=3.11\"\ndependencies = \
		 []\n\n[project.optional-dependencies]\nasync = [\"httpx>=0.27\"]\nlive = \
		 [\"websockets>=13\"]\n\n[tool.hatch.build.targets.wheel]\npackages = [{module}]\n",
		banner = context.banner("#"),
		name = crate::naming::toml_string(&context.options.package_name),
		version = crate::naming::toml_string(&context.version),
		description = crate::naming::toml_string(&format!(
			"A typed client for the {} RPC.",
			context.spec.title
		)),
		module = crate::naming::toml_string(module),
	)
}

fn readme(context: &Context<'_>, module: &str, methods: &[String], channels: &[String]) -> String {
	let example =
		context
			.functions()
			.zip(methods)
			.next()
			.map_or_else(String::new, |(function, method)| {
				let args: Vec<String> = function
					.params
					.iter()
					.zip(parameter_names(function))
					.filter(|(param, _)| param.is_required())
					.map(|(_, identifier)| format!("{identifier}=..."))
					.collect();
				format!("result = api.{method}({})\n", args.join(", "))
			});
	let live = live_readme(context, module, methods, channels);
	let mut skipped = String::new();

	if !context.skipped.is_empty() {
		skipped.push_str("\n## Not generated\n\n");

		for (name, reason) in &context.skipped {
			skipped.push_str(&format!("- `{name}`: {reason}\n"));
		}
	}

	format!(
		"# {name}

A typed Python client for the **{title} {version}** RPC, generated by `leptoskit rpc generate \
		 --target python`.

The synchronous `Client` uses only the standard library; `AsyncClient` needs the `async` extra \
		 (`pip install '{name}[async]'`), which brings in `httpx`. Models are `TypedDict`s, so \
		 values are plain dictionaries, and the calls go to the same routes the app itself uses.

## Usage

```python
from {module} import Client, FnError

api = Client({base_url})

try:
    {example}except FnError as error:
    if error.retryable:
        ...  # a network or server failure: safe to try again
    raise
```

```python
from {module} import AsyncClient

async with AsyncClient({base_url}) as api:
    {example}```

`FnError.kind` is one of `network`, `invalid`, `server`, `decode`, or `other`, mirroring the \
		 server's classification; `retryable` is true only for `network`.\n{live}{skipped}",
		name = context.options.package_name,
		title = context.spec.title,
		version = context.spec.version,
		base_url = py_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The README's live section: what the socket gives, with the first query,
/// mutation and channel as examples.
fn live_readme(
	context: &Context<'_>,
	module: &str,
	methods: &[String],
	channels: &[String],
) -> String {
	let example = |kind: ApiKind| {
		context
			.functions()
			.zip(methods)
			.find(|(function, _)| function.kind == kind)
	};
	let arguments = |function: &ApiFunction| {
		function
			.params
			.iter()
			.zip(parameter_names(function))
			.filter(|(param, _)| param.is_required())
			.map(|(_, identifier)| format!("{identifier}=..."))
			.collect::<Vec<_>>()
			.join(", ")
	};
	let mut lines = Vec::new();

	if let Some((function, method)) = example(ApiKind::Query) {
		lines.push(format!(
			"with api.{method}.watch({}) as watch:\n    for value in watch:\n        render(value)",
			arguments(function)
		));
	}

	if let Some((function, method)) = example(ApiKind::Mutation) {
		lines.push(format!("api.{method}.live({})", arguments(function)));
	}

	if let Some(member) = channels.first() {
		lines.push(format!(
			"room = api.{member}.join(\n    {{...}},\n    presence={{...}},\n    on_signal=lambda \
			 message, sender: show(message),\n    on_presence=lambda members: \
			 show_members(members),\n)\nroom.signal(...)"
		));
	}

	if lines.is_empty() {
		return String::new();
	}

	format!(
		"\n## Live\n\nQueries, mutations and channels also run over one WebSocket to the server's \
		 `{path}`, opened on first use. It needs the `live` extra (`pip install '{name}[live]'`), \
		 which brings in `websockets`:\n\n```python\nfrom {module} import Client, \
		 LiveOptions\n\napi = Client({base_url}, live=LiveOptions(token=lambda: \
		 access_token))\n\n{examples}\n```\n\nA watch yields the first value and every change; \
		 lists of rows change by patch, so a long list costs about one row per update. On \
		 `AsyncClient` the same calls are `async for value in api.<query>.watch(...)` and `await \
		 api.<mutation>.live(...)`. `.live(...)` returns once your own watches show the write, or \
		 after `read_your_writes` (2 seconds by default) if none of them changes. The connection \
		 reconnects with backoff, subscribes again, and re-sends a mutation whose answer was lost \
		 with the same key, so it runs once. Read `api.live.status` or pass a listener to \
		 `api.live.on_status(...)` for a connection banner, and call `api.live.close()` when \
		 done.\n",
		path = context.spec.live_path,
		name = context.options.package_name,
		base_url = py_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
		examples = lines.join("\n\n"),
	)
}

/// The transport, error type, and `serde_qs` encoder every client shares.
const RUNTIME: &str = r#"
class FnError(Exception):
    """A failed RPC call, classified like ``leptoskit_rpc::FnError``.

    ``kind`` is one of ``network``, ``invalid``, ``server``, ``decode``, or ``other``.
    """

    def __init__(
        self,
        kind: str,
        message: str,
        *,
        status: Optional[int] = None,
        raw: Optional[str] = None,
    ) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.status = status
        self.raw = raw

    @property
    def retryable(self) -> bool:
        """Whether retrying the call could plausibly succeed: only when no answer arrived.

        Server errors are final: the server reports a refusal such as "not found" the same way
        as an outage.
        """
        return self.kind == "network"

    @classmethod
    def from_response(cls, status: int, body: str) -> "FnError":
        """Parses leptos's error body: ``Kind|message`` as ``text/plain``."""
        variant, separator, message = body.partition("|")
        if not separator:
            return cls("other", body or f"HTTP {status}", status=status, raw=body)
        kind = _ERROR_KINDS.get(variant, "other")
        return cls(kind, message, status=status, raw=body)


_ERROR_KINDS = {
    "Request": "network",
    "Args": "invalid",
    "MissingArg": "invalid",
    "ServerError": "server",
    "MiddlewareError": "server",
    "WrappedServerFn": "server",
    "Deserialization": "decode",
    "Serialization": "decode",
}


def to_query_string(value: Mapping[str, Any]) -> str:
    """Encodes a value the way ``serde_qs`` does.

    Nested keys go in brackets, lists by index, and ``None`` is omitted.
    """
    pairs: list[tuple[str, str]] = []

    def append(key: str, item: Any) -> None:
        if item is None:
            return
        if isinstance(item, bool):
            pairs.append((key, "true" if item else "false"))
        elif isinstance(item, (list, tuple)):
            for index, entry in enumerate(item):
                append(f"{key}[{index}]", entry)
        elif isinstance(item, Mapping):
            for field, entry in item.items():
                append(f"{key}[{field}]", entry)
        else:
            pairs.append((key, str(item)))

    for key, item in value.items():
        append(key, item)
    return urllib.parse.urlencode(pairs)


def _decode(status: int, body: str) -> Any:
    if body == "":
        return None
    try:
        return json.loads(body)
    except ValueError as error:
        raise FnError("decode", f"the response was not JSON: {error}", status=status, raw=body) from error


class _BaseClient:
    def __init__(
        self,
        base_url: str = DEFAULT_SERVER_URL,
        *,
        headers: Optional[Mapping[str, str]] = None,
        timeout: float = 30.0,
        live: Optional[LiveOptions] = None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.headers = dict(headers or {})
        self.timeout = timeout
        self.live_options = live or LiveOptions()
        self._live: Any = None

    def _prepare(
        self,
        method: str,
        path: str,
        encoding: str,
        payload: Mapping[str, Any],
        idempotency_key: Optional[str] = None,
    ) -> tuple[str, dict[str, str], Optional[bytes]]:
        url = f"{self.base_url}{path}"
        headers = {"Accept": "application/json", **self.headers}
        if idempotency_key is not None:
            # A repeat with the same key and arguments returns the first
            # call's result instead of running the mutation again.
            headers["Idempotency-Key"] = idempotency_key
        data: Optional[bytes] = None
        if encoding == "query":
            query = to_query_string(payload)
            if query:
                url += ("&" if "?" in url else "?") + query
        elif encoding == "form":
            headers["Content-Type"] = "application/x-www-form-urlencoded"
            data = to_query_string(payload).encode("utf-8")
        else:
            headers["Content-Type"] = "application/json"
            # Absent optional arguments are omitted, as `serde_qs` omits them.
            present = {key: item for key, item in payload.items() if item is not None}
            data = json.dumps(present).encode("utf-8")
        return url, headers, data

"#;
