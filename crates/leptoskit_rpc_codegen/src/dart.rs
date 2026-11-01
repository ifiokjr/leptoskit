//! The Dart client: a package with typed models over `package:http`.
//!
//! Models are ordinary classes with `fromJson`/`toJson` generated from the
//! schemas, string enums become Dart enums, and shapes Dart cannot model yet
//! (tagged unions) arrive as `Object?` holding the decoded JSON. Calls go to
//! the same routes the app itself uses. Queries, mutations and channels
//! also run over the live socket, through the runtime in `lib/src/live.dart`.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashMap;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::DART_RESERVED_FIELDS;
use crate::naming::DART_RESERVED_METHODS;
use crate::naming::DART_RESERVED_PARAMETERS;
use crate::naming::DART_RESERVED_TYPES;
use crate::naming::Scope;
use crate::naming::component_names;
use crate::naming::dart_identifier;
use crate::naming::dart_package_name;
use crate::naming::dart_string;
use crate::naming::type_name;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiFunction;
use crate::spec::ApiKind;
use crate::spec::ApiSpec;

/// Whether a named type is a class or an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NamedKind {
	Class,
	Enum,
}

/// A Dart type, carrying just enough to encode and decode it.
#[derive(Debug, Clone, PartialEq)]
enum DartType {
	/// The `()` of a function returning nothing.
	Void,
	Bool,
	Int,
	Double,
	String,
	Named(String, NamedKind),
	List(Box<DartType>),
	Map(Box<DartType>),
	Dynamic,
	Nullable(Box<DartType>),
}

impl DartType {
	fn render(&self) -> String {
		match self {
			Self::Void => "void".to_owned(),
			Self::Bool => "bool".to_owned(),
			Self::Int => "int".to_owned(),
			Self::Double => "double".to_owned(),
			Self::String => "String".to_owned(),
			Self::Named(name, _) => name.clone(),
			Self::List(inner) => format!("List<{}>", inner.render()),
			Self::Map(inner) => format!("Map<String, {}>", inner.render()),
			Self::Dynamic => "Object?".to_owned(),
			Self::Nullable(inner) => format!("{}?", inner.render()),
		}
	}

	/// An expression converting the JSON value `expr` into this type.
	fn decode(&self, expr: &str) -> String {
		match self {
			Self::Void | Self::Dynamic => expr.to_owned(),
			Self::Bool => format!("{expr} as bool"),
			Self::Int => format!("({expr} as num).toInt()"),
			Self::Double => format!("({expr} as num).toDouble()"),
			Self::String => format!("{expr} as String"),
			Self::Named(name, NamedKind::Class) => {
				format!("{name}.fromJson({expr} as Map<String, dynamic>)")
			}
			Self::Named(name, NamedKind::Enum) => format!("{name}.fromJson({expr})"),
			Self::List(inner) => {
				format!(
					"({expr} as List<dynamic>).map((item) => {}).toList()",
					inner.decode("item")
				)
			}
			Self::Map(inner) => {
				format!(
					"({expr} as Map<String, dynamic>).map((key, value) => MapEntry(key, {}))",
					inner.decode("value")
				)
			}
			Self::Nullable(inner) => {
				format!("{expr} == null ? null : {}", inner.decode(expr))
			}
		}
	}

	/// An expression converting the Dart value `expr` into JSON.
	fn encode(&self, expr: &str) -> String {
		match self {
			Self::Void | Self::Dynamic | Self::Bool | Self::Int | Self::Double | Self::String => {
				expr.to_owned()
			}
			Self::Named(..) => format!("{expr}.toJson()"),
			Self::List(inner) => {
				let item = inner.encode("item");
				if item == "item" {
					expr.to_owned()
				} else {
					format!("{expr}.map((item) => {item}).toList()")
				}
			}
			Self::Map(inner) => {
				let value = inner.encode("value");
				if value == "value" {
					expr.to_owned()
				} else {
					format!("{expr}.map((key, value) => MapEntry(key, {value}))")
				}
			}
			Self::Nullable(inner) => inner.encode(expr),
		}
	}
}

/// Accumulates the generated classes and enums, and the names already taken.
struct DartTypes {
	scope: Scope,
	/// The bound name of every component schema, by schema name.
	components: HashMap<String, String>,
	/// Whether each declared type, by its Dart name, is a class or an enum.
	kinds: HashMap<String, NamedKind>,
	/// Emitted model declarations, in creation order.
	models: Vec<String>,
}

impl DartTypes {
	/// A table with every component's name bound up front, so references
	/// and synthesized types cannot collide with them.
	fn new(spec: &ApiSpec) -> Self {
		let mut scope = Scope::new(DART_RESERVED_TYPES);
		let components = component_names(spec, &mut scope);
		Self {
			scope,
			components,
			kinds: HashMap::new(),
			models: Vec::new(),
		}
	}

	fn reserve(&mut self, wanted: &str) -> String {
		self.scope.bind(wanted)
	}

	/// The Dart type for a schema, synthesizing classes and enums for
	/// anonymous shapes, named after `hint`.
	fn convert(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> DartType {
		let analyzed = analyze(schema);
		let nullable = analyzed.nullable;

		let base = match analyzed.shape {
			Shape::Ref(name) => {
				match (self.components.get(name), spec.schema(name)) {
					(Some(dart_name), Some(component)) => {
						match self.kinds.get(dart_name) {
							Some(kind) => DartType::Named(dart_name.clone(), *kind),
							// Aliases and maps have no class of their own.
							None => self.convert(component, &dart_name.clone(), spec),
						}
					}
					_ => DartType::Dynamic,
				}
			}
			Shape::Null => DartType::Void,
			Shape::Boolean => DartType::Bool,
			Shape::Integer { .. } => DartType::Int,
			Shape::Number { .. } => DartType::Double,
			Shape::String {
				values: Some(_), ..
			} => {
				let name = self.reserve(&type_name(hint));
				let declaration = enum_declaration(schema, &name);
				self.kinds.insert(name.clone(), NamedKind::Enum);
				self.models.push(declaration);
				DartType::Named(name, NamedKind::Enum)
			}
			Shape::String { .. } => DartType::String,
			Shape::Array { items } => {
				DartType::List(Box::new(items.map_or(DartType::Dynamic, |items| {
					self.convert(items, &format!("{hint}Item"), spec)
				})))
			}
			Shape::Tuple(_) => DartType::List(Box::new(DartType::Dynamic)),
			Shape::Object(object) => {
				if object.properties.is_empty() {
					DartType::Map(Box::new(
						object.additional.map_or(DartType::Dynamic, |values| {
							self.convert(values, &format!("{hint}Value"), spec)
						}),
					))
				} else {
					let name = self.reserve(&type_name(hint));
					let declaration = class_declaration(schema, &name, &object, self, spec);
					self.kinds.insert(name.clone(), NamedKind::Class);
					self.models.push(declaration);
					DartType::Named(name, NamedKind::Class)
				}
			}
			Shape::Map { values } => {
				DartType::Map(Box::new(values.map_or(DartType::Dynamic, |values| {
					self.convert(values, &format!("{hint}Value"), spec)
				})))
			}
			Shape::Union(_) | Shape::Any => DartType::Dynamic,
		};

		// `null` alone is `void`, not an optional nothing.
		if nullable && base != DartType::Void {
			DartType::Nullable(Box::new(base))
		} else {
			base
		}
	}
}

/// One field of a generated class.
struct Field {
	dart: String,
	wire: String,
	ty: DartType,
	nullable: bool,
	docs: Vec<String>,
}

/// The `class` declaration for an object schema.
fn class_declaration(
	schema: &Value,
	name: &str,
	object: &crate::schema::ObjectShape<'_>,
	types: &mut DartTypes,
	spec: &ApiSpec,
) -> String {
	let fields = class_fields(object, types, spec, name);
	let mut out = String::new();
	push_docs(&mut out, description(schema), "");
	out.push_str(&format!("class {name} {{\n"));
	out.push_str(&constructor(name, &fields));
	out.push_str(&field_lines(&fields));
	out.push_str(&from_json(name, &fields));
	out.push_str(&to_json(&fields));
	out.push_str("}\n\n");
	out
}

/// Each property's Dart field, named within the class.
fn class_fields(
	object: &crate::schema::ObjectShape<'_>,
	types: &mut DartTypes,
	spec: &ApiSpec,
	name: &str,
) -> Vec<Field> {
	let mut fields = Vec::new();
	let mut field_names = Scope::new(DART_RESERVED_FIELDS);

	for (wire, property) in &object.properties {
		let ty = types.convert(property, &format!("{name}{}", type_name(wire)), spec);
		let nullable = !object.is_required(wire)
			|| has_default(property)
			|| matches!(ty, DartType::Nullable(_));
		fields.push(Field {
			dart: field_names.bind(&dart_identifier(wire)),
			wire: (*wire).to_owned(),
			ty,
			nullable,
			docs: description(property)
				.map(|docs| docs.lines().map(str::trim).map(str::to_owned).collect())
				.unwrap_or_default(),
		});
	}

	fields
}

/// The constructor's named parameters.
fn constructor(name: &str, fields: &[Field]) -> String {
	format!(
		"\t{name}({{\n{}\n\t}});\n\n",
		fields
			.iter()
			.map(|field| {
				if field.nullable {
					format!("\t\tthis.{},", field.dart)
				} else {
					format!("\t\trequired this.{},", field.dart)
				}
			})
			.collect::<Vec<_>>()
			.join("\n")
	)
}

/// Each field's declaration, with its docs.
fn field_lines(fields: &[Field]) -> String {
	let mut out = String::new();

	for field in fields {
		let ty = if field.nullable {
			DartType::Nullable(Box::new(field.ty.clone()))
		} else {
			field.ty.clone()
		};

		for line in &field.docs {
			out.push_str(&format!("\t/// {line}\n"));
		}

		out.push_str(&format!("\tfinal {} {};\n", ty.render(), field.dart));
	}

	out
}

/// The `fromJson` factory.
fn from_json(name: &str, fields: &[Field]) -> String {
	format!(
		"\n\tfactory {name}.fromJson(Map<String, dynamic> json) => {name}(\n{}\n\t);\n",
		fields
			.iter()
			.map(|field| {
				let ty = if field.nullable {
					DartType::Nullable(Box::new(field.ty.clone()))
				} else {
					field.ty.clone()
				};

				format!(
					"\t\t{}: {},",
					field.dart,
					ty.decode(&format!("json[{}]", dart_string(&field.wire)))
				)
			})
			.collect::<Vec<_>>()
			.join("\n")
	)
}

/// The `toJson` method.
fn to_json(fields: &[Field]) -> String {
	format!(
		"\n\tMap<String, Object?> toJson() => <String, Object?>{{\n{}\n\t}};\n",
		fields
			.iter()
			.map(|field| {
				let entry = if field.nullable {
					DartType::Nullable(Box::new(field.ty.clone())).encode(&field.dart)
				} else {
					field.ty.encode(&field.dart)
				};
				if field.nullable {
					format!(
						"\t\tif ({0} != null) {1}: {2},",
						field.dart,
						dart_string(&field.wire),
						entry
					)
				} else {
					format!("\t\t{}: {},", dart_string(&field.wire), entry)
				}
			})
			.collect::<Vec<_>>()
			.join("\n")
	)
}

/// The `enum` declaration for a string-enum schema.
fn enum_declaration(schema: &Value, name: &str) -> String {
	let values: Vec<&str> = match analyze(schema).shape {
		Shape::String {
			values: Some(values),
			..
		} => values,
		_ => Vec::new(),
	};

	let mut out = String::new();
	push_docs(&mut out, description(schema), "");
	out.push_str(&format!("enum {name} {{\n"));
	let mut variants = Vec::new();

	for (index, value) in values.iter().enumerate() {
		let variant = dart_identifier(value);
		// Enum members Dart defines, which a value may not shadow.
		let variant = if ["values", "wire", "index", "name", "toJson", "fromJson"]
			.contains(&variant.as_str())
		{
			format!("{variant}_")
		} else {
			variant
		};
		// Values differing only in case still need distinct variant names.
		let variant = if values
			.iter()
			.enumerate()
			.any(|(other, candidate)| other != index && dart_identifier(candidate) == variant)
		{
			format!("{variant}{index}")
		} else {
			variant
		};
		variants.push(format!("\t{}({})", variant, dart_string(value)));
	}

	out.push_str(&variants.join(",\n"));
	out.push_str(";\n");
	out.push_str(&format!(
		"\tconst {name}(this.wire);\n\n\tfinal String wire;\n\n\tfactory {name}.fromJson(Object? \
		 json) => values.firstWhere((value) => value.wire == json);\n\n\tString toJson() => \
		 wire;\n}}\n\n",
	));
	out
}

/// Pushes a `///` doc comment at `indent`.
fn push_docs(out: &mut String, docs: Option<&str>, indent: &str) {
	let Some(docs) = docs else {
		return;
	};
	let docs = docs.trim();

	if docs.is_empty() {
		return;
	}

	let safe = docs.replace("*/", "*\\/");

	for line in safe.lines() {
		if line.is_empty() {
			out.push_str(&format!("{indent}///\n"));
		} else {
			out.push_str(&format!("{indent}/// {line}\n"));
		}
	}
}

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let package = dart_package_name(&context.options.package_name);
	let mut types = DartTypes::new(context.spec);
	let components: Vec<(String, &Value)> = context
		.spec
		.schemas
		.iter()
		.map(|(name, schema)| {
			let dart_name = types.components[name].clone();
			let kind = match analyze(schema).shape {
				Shape::Object(ref object) if !object.properties.is_empty() => {
					Some(NamedKind::Class)
				}

				Shape::String {
					values: Some(_), ..
				} => Some(NamedKind::Enum),
				_ => None,
			};
			if let Some(kind) = kind {
				types.kinds.insert(dart_name.clone(), kind);
			}

			(dart_name, schema)
		})
		.collect();

	let mut models = String::new();

	for (dart_name, schema) in &components {
		match analyze(schema).shape {
			Shape::Object(ref object) if !object.properties.is_empty() => {
				models.push_str(&class_declaration(
					schema,
					dart_name,
					object,
					&mut types,
					context.spec,
				));
			}

			Shape::String {
				values: Some(_), ..
			} => {
				models.push_str(&enum_declaration(schema, dart_name));
			}
			_ => {}
		}
	}

	// The client's signatures may synthesize more models (inline enums,
	// anonymous argument objects), so it is rendered before the models are
	// written out.
	let (methods, channels) = member_names(context);
	let client = client_class(context, &mut types, &methods, &channels);
	let mut library = context.banner("//");
	library.push_str("library;\n\nimport 'dart:convert';\nimport 'dart:typed_data';\n\n");
	library.push_str("import 'package:http/http.dart' as http;\n\n");
	// The live runtime is its own library next to this one, and everything
	// it declares is part of the package's API.
	library.push_str("import 'src/live.dart';\n\nexport 'src/live.dart';\n\n");
	library.push_str(&models);

	for synthesized in &types.models {
		library.push_str(synthesized);
	}

	library.push_str(RUNTIME);
	library.push_str(&client);
	let mut live = context.banner("//");
	live.push_str(include_str!("templates/live.dart"));

	Generated {
		files: vec![
			GeneratedFile::scaffold("pubspec.yaml", pubspec(context, &package)),
			GeneratedFile::code(format!("lib/{package}.dart"), library),
			GeneratedFile::code("lib/src/live.dart", live),
			GeneratedFile::scaffold("README.md", readme(context, &methods, &channels)),
		],
	}
}

/// The member name of every callable function and then every channel, in
/// order, clear of `Client`'s own members and of each other.
fn member_names(context: &Context<'_>) -> (Vec<String>, Vec<String>) {
	let mut scope = Scope::new(DART_RESERVED_METHODS);
	let methods = context
		.functions()
		.map(|function| scope.bind(&dart_identifier(&function.name)))
		.collect();
	let channels = context
		.spec
		.channels
		.iter()
		.map(|channel| scope.bind(&dart_identifier(&channel.name)))
		.collect();
	(methods, channels)
}

/// The named parameter of every argument, in order, clear of the method
/// body's locals.
fn parameter_names(function: &ApiFunction) -> Vec<String> {
	let mut scope = Scope::new(DART_RESERVED_PARAMETERS);
	function
		.params
		.iter()
		.map(|param| scope.bind(&dart_identifier(&param.name)))
		.collect()
}

/// What every form of a function shares: its result, its parameter list,
/// and the payload map the arguments build.
struct Signature {
	output: DartType,
	/// The named parameter list, `idempotencyKey` included for a mutation.
	parameters: String,
	payload: String,
	/// The idempotency key argument a mutation passes on.
	key: &'static str,
}

impl Signature {
	fn new(function: &ApiFunction, types: &mut DartTypes, spec: &ApiSpec) -> Self {
		let identifiers = parameter_names(function);
		let output = types.convert(
			&function.output_schema,
			&format!("{}Result", type_name(&function.name)),
			spec,
		);

		let mut parameters = Vec::new();
		let mut payload = Vec::new();

		for (param, identifier) in function.params.iter().zip(&identifiers) {
			let ty = types.convert(
				&param.schema,
				&format!("{}{}", type_name(&function.name), type_name(&param.name)),
				spec,
			);
			// The parameter type is what the wire accepts; an optional
			// non-nullable argument becomes nullable once, not twice.
			let optional = !param.is_required() || matches!(ty, DartType::Nullable(_));

			let parameter_type = match (&ty, optional) {
				(DartType::Nullable(_), _) | (_, false) => ty.clone(),
				(_, true) => DartType::Nullable(Box::new(ty.clone())),
			};
			parameters.push(if optional {
				format!("{} {}", parameter_type.render(), identifier)
			} else {
				format!("required {} {}", parameter_type.render(), identifier)
			});

			if optional {
				payload.push(format!(
					"\t\t\t\tif ({identifier} != null) {}: {},",
					dart_string(&param.name),
					ty.encode(identifier)
				));
			} else {
				payload.push(format!(
					"\t\t\t\t{}: {},",
					dart_string(&param.name),
					ty.encode(identifier)
				));
			}
		}

		// A mutation takes an idempotency key, so a retried call runs once.
		let key = if function.kind.is_idempotent_with_key() {
			parameters.push("String? idempotencyKey".to_owned());
			", idempotencyKey: idempotencyKey"
		} else {
			""
		};

		let parameters = if parameters.is_empty() {
			String::new()
		} else {
			format!("{{\n\t\t{}\n\t}}", parameters.join(",\n\t\t"))
		};
		let payload = if payload.is_empty() {
			"const <String, Object?>{}".to_owned()
		} else {
			format!(
				"\n\t\t\t\t<String, Object?>{{\n{}\n\t\t\t\t}}",
				payload.join("\n")
			)
		};

		Self {
			output,
			parameters,
			payload,
			key,
		}
	}

	fn returns_nothing(&self) -> bool {
		matches!(self.output, DartType::Void)
	}

	/// `Future<T>` of the result.
	fn future(&self) -> String {
		if self.returns_nothing() {
			"Future<void>".to_owned()
		} else {
			format!("Future<{}>", self.output.render())
		}
	}

	/// An `async` method `name` that sends the arguments through `send`,
	/// such as `_invoke('POST', '/api/x', 'json', `, and decodes the result.
	fn method(&self, name: &str, send: &str) -> String {
		let (binding, decode) = if self.returns_nothing() {
			("", String::new())
		} else {
			(
				"final result = ",
				format!("\n\t\treturn {};", self.output.decode("result")),
			)
		};
		format!(
			"\t{} {name}({}) async {{\n\t\t{binding}await {send}{}{});{decode}\n\t}}\n",
			self.future(),
			self.parameters,
			self.payload,
			self.key,
		)
	}
}

/// A function's doc comment: its docs, then its documented arguments.
fn function_docs(function: &ApiFunction) -> String {
	let mut docs = function.docs.clone();
	let documented: Vec<(&crate::spec::ApiParam, String)> = function
		.params
		.iter()
		.zip(parameter_names(function))
		.filter(|(param, _)| !param.docs.is_empty())
		.collect();

	if !documented.is_empty() {
		if !docs.is_empty() {
			docs.push_str("\n\n");
		}

		docs.push_str("Arguments:\n");

		for (param, identifier) in documented {
			docs.push_str(&format!("- `{identifier}`: {}\n", param.docs.trim()));
		}

		docs.pop();
	}

	if function.deprecated {
		if docs.is_empty() {
			docs.push_str("Deprecated.");
		} else {
			docs.push_str("\n\nDeprecated.");
		}
	}

	docs
}

/// The generated `Client`, with one member per published function and
/// channel, followed by the classes behind the live members.
fn client_class(
	context: &Context<'_>,
	types: &mut DartTypes,
	methods: &[String],
	channels: &[String],
) -> String {
	let mut out = String::new();
	let mut members = String::new();
	out.push_str(&format!(
		"/// The server origin baked in at generation time; `null` means the client's own \
		 page.\nconst String? defaultServerUrl = {};\n\n/// Where the server serves the live \
		 socket, relative to its origin.\nconst String livePath = {};\n\n",
		context
			.server_url
			.as_deref()
			.map_or_else(|| "null".to_owned(), dart_string),
		dart_string(&context.spec.live_path),
	));
	out.push_str("/// A typed client over `package:http`, with a live side over one WebSocket.\n");
	out.push_str("class Client {\n");
	out.push_str(CLIENT_FIELDS);

	for (function, name) in context.functions().zip(methods) {
		out.push('\n');
		let docs = function_docs(function);
		push_docs(&mut out, (!docs.is_empty()).then_some(docs.as_str()), "\t");
		let signature = Signature::new(function, types, context.spec);
		let invoke = format!(
			"_invoke({}, {}, {}, ",
			dart_string(&function.method),
			dart_string(&function.path),
			dart_string(match function.input {
				crate::spec::HttpInput::Query => "query",
				crate::spec::HttpInput::Form => "form",
				crate::spec::HttpInput::Json | crate::spec::HttpInput::Other { .. } => "json",
			}),
		);

		if !matches!(function.kind, ApiKind::Query | ApiKind::Mutation) {
			out.push_str(&signature.method(name, &invoke));
			continue;
		}

		// A query or mutation is an object: calling it makes the one-shot
		// HTTP call, as before, and its methods run it over the socket.
		let suffix = if function.kind == ApiKind::Query {
			"Query"
		} else {
			"Mutation"
		};
		let class = types.reserve(&format!("{}{suffix}", type_name(&function.name)));
		out.push_str(&format!("\tlate final {class} {name} = {class}._(this);\n"));
		members.push_str(&live_class(function, &class, &signature, &invoke));
	}

	for (channel, name) in context.spec.channels.iter().zip(channels) {
		out.push('\n');
		let mut docs = channel.docs.clone();

		if channel.deprecated {
			docs.push_str(if docs.is_empty() {
				"Deprecated."
			} else {
				"\n\nDeprecated."
			});
		}

		push_docs(&mut out, (!docs.is_empty()).then_some(docs.as_str()), "\t");
		let class = types.reserve(&format!("{}Channel", type_name(&channel.name)));
		out.push_str(&format!("\tlate final {class} {name} = {class}._(this);\n"));
		members.push_str(&channel_class(channel, &class, name, types, context.spec));
	}

	out.push_str(INVOKE);
	out.push_str(&members);
	out
}

/// The class behind a query or mutation member: `call` for the one-shot
/// HTTP call, with `watch` for a query and `live` for a mutation.
fn live_class(function: &ApiFunction, class: &str, signature: &Signature, invoke: &str) -> String {
	let name = dart_string(&function.name);
	let mut out = format!(
		"\n/// The `{}` {}: call it for one answer over HTTP, or {}\n/// over the live \
		 connection.\nclass {class} {{\n\t{class}._(this._client);\n\n\tfinal Client \
		 _client;\n\n\t/// Calls it once over HTTP.\n",
		function.name,
		function.kind.as_str(),
		if function.kind == ApiKind::Query {
			"[watch] it"
		} else {
			"run it [live]"
		},
	);
	out.push_str(&signature.method("call", &format!("_client.{invoke}")));
	out.push('\n');

	if function.kind == ApiKind::Query {
		let (ty, decode) = if signature.returns_nothing() {
			("void".to_owned(), "(result) {}".to_owned())
		} else {
			(
				signature.output.render(),
				format!("(result) => {}", signature.output.decode("result")),
			)
		};
		out.push_str(&format!(
			"\t/// Watches the result over the live connection: the stream gets the first\n\t/// \
			 value and each new one, ready for a `StreamBuilder`. A failure arrives as\n\t/// a \
			 [LiveError] error event, and the subscription recovers on the next\n\t/// change. \
			 Each listener gets its own subscription, until it cancels.\n\tStream<{ty}> watch({}) \
			 => _client.live.watch<{ty}>({name}, {}, {decode});\n",
			signature.parameters, signature.payload,
		));
	} else {
		out.push_str(
			"\t/// Runs it over the live connection. It completes once this client's\n\t/// \
			 subscriptions show the write, and a call whose answer is lost is sent\n\t/// again \
			 after a reconnect with the same key, so it runs once.\n",
		);
		out.push_str(&signature.method("live", &format!("_client.live.mutate({name}, ")));
	}

	out.push_str("}\n");
	out
}

/// The class behind a channel member: `join` returns the joined channel.
fn channel_class(
	channel: &crate::spec::ApiChannel,
	class: &str,
	member: &str,
	types: &mut DartTypes,
	spec: &ApiSpec,
) -> String {
	let hint = type_name(&channel.name);
	// A channel's values are JSON, never nothing.
	let mut convert = |schema: &Value, suffix: &str| {
		match types.convert(schema, &format!("{hint}{suffix}"), spec) {
			DartType::Void => DartType::Dynamic,
			other => other,
		}
	};
	let args = convert(&channel.args_schema, "Args");
	let message = convert(&channel.message_schema, "Message");
	let presence = convert(&channel.presence_schema, "Presence");
	let optional_presence = match presence {
		DartType::Nullable(_) | DartType::Dynamic => presence.clone(),
		_ => DartType::Nullable(Box::new(presence.clone())),
	};
	let fields = channel
		.fields
		.iter()
		.map(|field| dart_string(field))
		.collect::<Vec<_>>()
		.join(", ");
	let (message_type, presence_type) = (message.render(), presence.render());
	format!(
		"\n/// The `{channel_name}` channel; see [Client.{member}].\nclass {class} \
		 {{\n\t{class}._(this._client);\n\n\tfinal Client _client;\n\n\t/// Joins the channel \
		 instance [args] names: signals and presence arrive on\n\t/// the handle's streams, and \
		 the join is made again after a reconnect, until\n\t/// [ChannelHandle.leave]. [presence] \
		 is what this connection shows the others;\n\t/// the channel's default when \
		 absent.\n\tChannelHandle<{message_type}, {presence_type}> join({args_type} args, \
		 {{{optional_presence} presence}}) =>\n\t\t_client.live.join<{message_type}, \
		 {presence_type}>(\n\t\t\t{name},\n\t\t\tfields: const <String>[{fields}],\n\t\t\targs: \
		 {args_json},\n\t\t\tpresence: presence,\n\t\t\tdecodeMessage: (json) => \
		 {decode_message},\n\t\t\tencodeMessage: (message) => \
		 {encode_message},\n\t\t\tdecodePresence: (json) => \
		 {decode_presence},\n\t\t\tencodePresence: (state) => {encode_presence},\n\t\t);\n}}\n",
		channel_name = channel.name,
		name = dart_string(&channel.name),
		args_type = args.render(),
		optional_presence = optional_presence.render(),
		args_json = to_json_value(&args, "args"),
		decode_message = message.decode("json"),
		encode_message = to_json_value(&message, "message"),
		decode_presence = presence.decode("json"),
		encode_presence = to_json_value(&presence, "state"),
	)
}

/// An expression converting `expr` into JSON, guarding a nullable model.
fn to_json_value(ty: &DartType, expr: &str) -> String {
	match ty {
		DartType::Nullable(inner) => {
			let encoded = inner.encode(expr);
			if encoded == expr {
				encoded
			} else {
				format!("{expr} == null ? null : {encoded}")
			}
		}
		_ => ty.encode(expr),
	}
}

/// The constructor and fields every `Client` opens with.
const CLIENT_FIELDS: &str =
	"\t/// Creates a client for [serverUrl], an origin such as\n\t/// `https://api.example.com`; \
	 defaults to [defaultServerUrl]. Pass\n\t/// [httpClient] to share connections, and [live] to \
	 sign the live\n\t/// connection in or tune its reconnects.\n\tClient({String? serverUrl, \
	 http.Client? httpClient, LiveOptions live = const LiveOptions()})\n\t\t : serverUrl = \
	 serverUrl ?? defaultServerUrl,\n\t\t   _http = httpClient ?? http.Client(),\n\t\t   \
	 liveOptions = live;\n\n\t/// The origin the client sends requests to, without a trailing \
	 slash.\n\tfinal String? serverUrl;\n\n\tfinal http.Client _http;\n\n\t/// How the live \
	 connection signs in and reconnects.\n\tfinal LiveOptions liveOptions;\n\n\tLiveConnection? \
	 _live;\n\n\t/// The live connection: one WebSocket for every subscription, live\n\t/// \
	 mutation and channel, opened on first use. Read [LiveConnection.status]\n\t/// or listen to \
	 [LiveConnection.statuses] for a connection banner, and\n\t/// call [LiveConnection.close] \
	 when done.\n\tLiveConnection get live =>\n\t\t_live ??= LiveConnection(liveUrl(serverUrl, \
	 livePath), liveOptions);\n";

fn pubspec(context: &Context<'_>, package: &str) -> String {
	format!(
		"{banner}name: {package}\ndescription: {description}\nversion: {version}\npublish_to: \
		 none\n\nenvironment:\n  sdk: '>=3.0.0 <4.0.0'\n\ndependencies:\n  http: ^1.2.0\n  \
		 web_socket_channel: '>=2.4.0 <4.0.0'\n",
		banner = context.banner("#"),
		description = serde_json::to_string(&format!(
			"A typed client for the {} RPC.",
			context.spec.title
		))
		.expect("strings always serialize"),
		version = serde_json::to_string(&context.version).expect("strings always serialize"),
	)
}

fn readme(context: &Context<'_>, methods: &[String], channels: &[String]) -> String {
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
					.map(|(_, identifier)| format!("{identifier}: ..."))
					.collect();
				format!(
					"final result = await client.{method}({});\n",
					args.join(", ")
				)
			});
	let live = live_readme(context, methods, channels);
	let mut skipped = String::new();

	if !context.skipped.is_empty() {
		skipped.push_str("\n## Not generated\n\n");

		for (name, reason) in &context.skipped {
			skipped.push_str(&format!("- `{name}`: {reason}\n"));
		}
	}

	format!(
		"# {name}

A typed Dart client for the **{title} {version}** RPC, generated by `leptoskit rpc generate \
		 --target dart`.

It depends on `package:http` and `package:web_socket_channel`, and the calls go to the same routes \
		 the app itself uses.

## Usage

```dart
import 'package:{package}/{package}.dart';

final client = Client(serverUrl: {server_url});

try {{
  {example}}} on FnError catch (error) {{
  if (error.retryable) {{
    // a network or server failure: safe to try again
  }}
  rethrow;
}}
```

`FnError.kind` is one of `network`, `invalid`, `server`, `decode`, or `other`, mirroring the \
		 server's classification; `retryable` is true only for `network`. Types that Dart cannot \
		 model yet (tagged unions) appear as `Object?` holding the raw JSON.\n{live}{skipped}",
		name = context.options.package_name,
		package = dart_package_name(&context.options.package_name),
		title = context.spec.title,
		version = context.spec.version,
		server_url = dart_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The README's live section: what the socket gives, with the first query,
/// mutation and channel as examples.
fn live_readme(context: &Context<'_>, methods: &[String], channels: &[String]) -> String {
	let example = |kind: ApiKind| {
		context
			.functions()
			.zip(methods)
			.find(|(function, _)| function.kind == kind)
	};
	// The required arguments, as `name: ...`.
	let arguments = |function: &ApiFunction| {
		function
			.params
			.iter()
			.zip(parameter_names(function))
			.filter(|(param, _)| param.is_required())
			.map(|(_, identifier)| format!("{identifier}: ..."))
			.collect::<Vec<_>>()
			.join(", ")
	};
	let mut lines = Vec::new();

	if let Some((function, method)) = example(ApiKind::Query) {
		lines.push(format!(
			"// A stream of results, ready for a StreamBuilder.\nfinal results = \
			 client.{method}.watch({});",
			arguments(function)
		));
	}

	if let Some((function, method)) = example(ApiKind::Mutation) {
		lines.push(format!(
			"// Completes once this client's subscriptions show the write.\nawait \
			 client.{method}.live({});",
			arguments(function)
		));
	}

	if let Some(member) = channels.first() {
		lines.push(format!(
			"final room = client.{member}.join(..., presence: ...);\nroom.signals.listen((signal) \
			 => show(signal.message, signal.from));\nroom.presence.listen((members) => \
			 list(members));\nroom.signal(...);\nroom.leave();"
		));
	}

	if lines.is_empty() {
		return String::new();
	}

	format!(
		"\n## Live\n\nQueries, mutations and channels also run over one WebSocket to the server's \
		 `{path}`, opened on first use:\n\n```dart\n{examples}\n```\n\nA query's `watch(...)` \
		 stream gets the first value and every change; lists of rows change by patch, so a long \
		 list costs about one row per update. A mutation's `live(...)` completes once your own \
		 subscriptions show the write, or after `readYourWrites` (2 seconds by default) if none \
		 of them changes. The connection reconnects with backoff, subscribes again, and re-sends \
		 a mutation whose answer was lost with the same key, so it runs once. Sign in with \
		 `Client(live: LiveOptions(token: () => accessToken))`; listen to `client.live.statuses` \
		 for a connection banner, and call `client.live.close()` when done. It works wherever \
		 `package:web_socket_channel` does: Flutter on every platform, the browser, and the Dart \
		 VM.\n",
		path = context.spec.live_path,
		examples = lines.join("\n"),
	)
}

/// The error type, `serde_qs` encoder, and helpers every package shares.
const RUNTIME: &str = r"/// How a failed call is classified, mirroring `leptoskit_rpc::FnError`.
class FnError implements Exception {
  /// One of `network`, `invalid`, `server`, `decode`, or `other`.
  final String kind;

  /// The human-readable message.
  final String message;

  /// The HTTP status, when a response arrived.
  final int? status;

  /// The response body, when a response arrived.
  final String? raw;

  FnError(this.kind, this.message, {this.status, this.raw});

  /// Whether retrying the call could plausibly succeed: only when no answer
  /// arrived. Server errors are final, because the server reports a refusal
  /// such as `not found` the same way as an outage.
  bool get retryable => kind == 'network';

  /// Parses leptos's error body: `Kind|message` as `text/plain`.
  factory FnError.fromResponse(int status, String body) {
    const kinds = <String, String>{
      'Request': 'network',
      'Args': 'invalid',
      'MissingArg': 'invalid',
      'ServerError': 'server',
      'MiddlewareError': 'server',
      'WrappedServerFn': 'server',
      'Deserialization': 'decode',
      'Serialization': 'decode',
    };
    final separator = body.indexOf('|');
    if (separator == -1) {
      final text = body.isEmpty ? 'HTTP $status' : body;
      return FnError('other', text, status: status, raw: body);
    }
    final variant = body.substring(0, separator);
    return FnError(
      kinds[variant] ?? 'other',
      body.substring(separator + 1),
      status: status,
      raw: body,
    );
  }

  @override
  String toString() => 'FnError($kind): $message';
}

/// Encodes a value the way `serde_qs` does: nested keys in brackets, lists by
/// index, `null` omitted.
String toQueryString(Map<String, Object?> value) {
  final pairs = <String>[];

  void append(String key, Object? item) {
    if (item == null) return;
    if (item is bool) {
      pairs.add('$key=${item ? 'true' : 'false'}');
    } else if (item is List) {
      for (var index = 0; index < item.length; index++) {
        append('$key[$index]', item[index]);
      }
    } else if (item is Map<String, Object?>) {
      for (final entry in item.entries) {
        append('$key[${entry.key}]', entry.value);
      }
    } else {
      pairs.add('$key=${Uri.encodeQueryComponent('$item')}');
    }
  }

  for (final entry in value.entries) {
    append(entry.key, entry.value);
  }
  return pairs.join('&');
}

";

/// The transport every generated method calls, closing the client class.
const INVOKE: &str =
	"\n\tFuture<Object?> _invoke(\n\t\tString method,\n\t\tString path,\n\t\tString \
	 encoding,\n\t\tMap<String, Object?> payload, {\n\t\tString? idempotencyKey,\n\t}) async \
	 {\n\t\tvar url = '${serverUrl ?? ''}$path';\n\t\tfinal headers = <String, String>{'Accept': \
	 'application/json'};\n\t\t// A repeat with the same key and arguments returns the first \
	 call's result\n\t\t// instead of running the mutation again.\n\t\tif (idempotencyKey != \
	 null) headers['Idempotency-Key'] = idempotencyKey;\n\t\tUint8List? body;\n\t\tif (encoding \
	 == 'query') {\n\t\t\tfinal query = toQueryString(payload);\n\t\t\tif (query.isNotEmpty) \
	 {\n\t\t\t\turl += (url.contains('?') ? '&' : '?') + query;\n\t\t\t}\n\t\t} else if (encoding \
	 == 'form') {\n\t\t\theaders['Content-Type'] = \
	 'application/x-www-form-urlencoded';\n\t\t\tbody = \
	 Uint8List.fromList(utf8.encode(toQueryString(payload)));\n\t\t} else \
	 {\n\t\t\theaders['Content-Type'] = 'application/json';\n\t\t\t// Absent optional arguments \
	 are omitted, as `serde_qs` omits them.\n\t\t\tfinal present = <String, \
	 Object?>{\n\t\t\t\tfor (final entry in payload.entries)\n\t\t\t\t\tif (entry.value != null) \
	 entry.key: entry.value,\n\t\t\t};\n\t\t\tbody = \
	 Uint8List.fromList(utf8.encode(jsonEncode(present)));\n\t\t}\n\t\tlate http.Response \
	 response;\n\t\ttry {\n\t\t\tfinal uri = Uri.parse(url);\n\t\t\tfinal request = \
	 http.Request(method, uri)\n\t\t\t\t..headers.addAll(headers);\n\t\t\tif (body != null) \
	 request.bodyBytes = body;\n\t\t\tresponse = await http.Response.fromStream(await \
	 _http.send(request));\n\t\t} catch (error) {\n\t\t\tthrow FnError('network', 'could not \
	 reach $url: $error');\n\t\t}\n\t\tif (response.statusCode >= 400) {\n\t\t\tthrow \
	 FnError.fromResponse(response.statusCode, response.body);\n\t\t}\n\t\tif \
	 (response.body.isEmpty) return null;\n\t\treturn jsonDecode(response.body);\n\t}\n}\n";
