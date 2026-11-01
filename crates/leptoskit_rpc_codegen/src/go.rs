//! The Go client: one package over `net/http`.
//!
//! Models are plain structs with `json` tags (so `encoding/json` round-trips
//! them unchanged), string enums become named string types with constants,
//! and anything Go cannot model yet (tagged unions) falls back to `any`
//! holding the decoded JSON. The wire encoding — `serde_qs` brackets for form
//! and query, JSON bodies — mirrors what the server's codecs read.

// Generators build source text; the allocation `write!` would save is noise here.
#![allow(clippy::format_push_string)]

use std::collections::HashMap;

use serde_json::Value;

use crate::generate::Context;
use crate::generate::Generated;
use crate::generate::GeneratedFile;
use crate::naming::GO_RESERVED;
use crate::naming::GO_RESERVED_METHODS;
use crate::naming::GO_RESERVED_PARAMETERS;
use crate::naming::Scope;
use crate::naming::component_names;
use crate::naming::go_identifier;
use crate::naming::go_package_name;
use crate::naming::go_string;
use crate::naming::type_name;
use crate::schema::ObjectShape;
use crate::schema::Shape;
use crate::schema::analyze;
use crate::schema::description;
use crate::schema::has_default;
use crate::spec::ApiFunction;
use crate::spec::ApiParam;
use crate::spec::ApiSpec;
use crate::spec::HttpInput;

pub(crate) fn generate(context: &Context<'_>) -> Generated {
	let package = go_package_name(&context.options.package_name);
	let mut types = GoTypes::new(context);
	let methods = method_names(context);
	let models = models_file(context, &mut types);
	let client = client_file(context, &package, &mut types, &methods);
	Generated {
		files: vec![
			GeneratedFile::scaffold("go.mod", go_mod(context)),
			GeneratedFile::code("models.go", models),
			GeneratedFile::code("client.go", client),
			GeneratedFile::scaffold("README.md", readme(context, &package, &methods)),
		],
	}
}

/// Tracks the package's names and the synthesized declarations for
/// anonymous shapes.
struct GoTypes {
	/// Every package-level name: models, synthesized types, enum constants.
	scope: Scope,
	/// The bound name of every component schema, by schema name.
	components: HashMap<String, String>,
	synthesized: Vec<String>,
}

impl GoTypes {
	/// A table with every component's name bound up front.
	fn new(context: &Context<'_>) -> Self {
		let mut scope = Scope::new(GO_RESERVED);
		let components = component_names(context.spec, &mut scope);
		Self {
			scope,
			components,
			synthesized: Vec::new(),
		}
	}

	fn reserve(&mut self, wanted: &str) -> String {
		self.scope.bind(wanted)
	}

	/// A Go type expression for `schema`, synthesizing structs for anonymous
	/// objects and named types for inline enums, named after `hint`.
	fn render(&mut self, schema: &Value, hint: &str, spec: &ApiSpec) -> String {
		let analyzed = analyze(schema);

		let base = match analyzed.shape {
			Shape::Ref(name) => {
				self.components
					.get(name)
					.cloned()
					.unwrap_or_else(|| "any".to_owned())
			}
			Shape::Null => "struct{}".to_owned(),
			Shape::Boolean => "bool".to_owned(),
			Shape::Integer { format } => integer_type(format),
			Shape::Number { .. } => "float64".to_owned(),
			Shape::String {
				values: Some(_), ..
			} => {
				let name = self.reserve(&type_name(hint));
				let declaration = enum_declaration(schema, &name, &mut self.scope);
				self.synthesized.push(declaration);
				name
			}
			Shape::String { .. } => "string".to_owned(),
			Shape::Array { items } => {
				let inner = items.map_or_else(
					|| "any".to_owned(),
					|items| self.render(items, &format!("{hint}Item"), spec),
				);
				// Slices and maps are nilable; Option only needs a pointer for
				// value types.
				format!("[]{inner}")
			}
			Shape::Tuple(_) => "[]any".to_owned(),
			Shape::Object(object) => {
				if object.properties.is_empty() {
					match object.additional {
						Some(values) => {
							format!(
								"map[string]{}",
								self.render(values, &format!("{hint}Value"), spec)
							)
						}
						None => "map[string]any".to_owned(),
					}
				} else {
					let name = self.reserve(&type_name(hint));
					let declaration =
						struct_declaration(&name, description(schema), &object, self, spec, &name);
					self.synthesized.push(declaration);
					name
				}
			}
			Shape::Map { values } => {
				format!(
					"map[string]{}",
					values.map_or_else(
						|| "any".to_owned(),
						|values| self.render(values, &format!("{hint}Value"), spec)
					)
				)
			}
			Shape::Union(_) | Shape::Any => "any".to_owned(),
		};

		if analyzed.nullable && needs_pointer(&base) {
			format!("*{base}")
		} else {
			base
		}
	}
}

/// Go type for an integer schema format.
pub(crate) fn integer_type(format: Option<&str>) -> String {
	match format {
		Some("int8") => "int8".to_owned(),
		Some("int16") => "int16".to_owned(),
		Some("int32") => "int32".to_owned(),
		Some("uint8") => "uint8".to_owned(),
		Some("uint16") => "uint16".to_owned(),
		Some("uint32") => "uint32".to_owned(),
		Some("uint64") => "uint64".to_owned(),
		_ => "int64".to_owned(),
	}
}

/// Whether an optional value of this type needs a pointer to express absence:
/// slices, maps, `any`, and structs (via a null in JSON) are nilable or
/// nullable already.
fn needs_pointer(type_name: &str) -> bool {
	!(type_name.starts_with("[]")
		|| type_name.starts_with("map[")
		|| type_name == "any"
		|| type_name == "struct{}")
}

/// One field of a generated struct.
struct Field {
	go_name: String,
	type_name: String,
	tag: String,
	docs: Vec<String>,
	/// `omitempty` for optional fields.
	optional: bool,
}

/// Encodes both the tag value and its enclosing Go source literal.
fn json_tag(wire: &str, optional: bool) -> String {
	let omit = if optional { ",omitempty" } else { "" };
	let tag = format!("json:{}", go_string(&format!("{wire}{omit}")));

	// Escaping the value protects quotes and controls; a backtick also needs
	// an interpreted outer literal so it cannot end a raw Go string.
	if tag.contains('`') {
		go_string(&tag)
	} else {
		format!("`{tag}`")
	}
}

/// A struct declaration, with gofmt-style tag alignment.
fn struct_declaration(
	name: &str,
	docs: Option<&str>,
	object: &ObjectShape<'_>,
	types: &mut GoTypes,
	spec: &ApiSpec,
	hint: &str,
) -> String {
	let mut fields = Vec::new();
	let mut field_names = Scope::new(&[]);

	for (wire, schema) in &object.properties {
		let field_type = types.render(schema, &format!("{hint}{}", type_name(wire)), spec);
		let optional = !object.is_required(wire) || has_default(schema);
		fields.push(Field {
			go_name: field_names.bind(&type_name(wire)),
			type_name: field_type,
			tag: (*wire).to_owned(),
			docs: description(schema)
				.map(|docs| docs.lines().map(str::trim).map(str::to_owned).collect())
				.unwrap_or_default(),
			optional,
		});
	}

	let mut out = String::new();

	for line in docs.unwrap_or_default().lines() {
		out.push_str(&format!("// {line}\n"));
	}

	out.push_str(&format!("type {name} struct {{\n"));
	let width = fields
		.iter()
		.map(|field| field.go_name.len())
		.max()
		.unwrap_or_default();
	let type_width = fields
		.iter()
		.map(|field| field.type_name.len())
		.max()
		.unwrap_or_default();

	for field in &fields {
		for line in &field.docs {
			out.push_str(&format!("\t// {line}\n"));
		}

		out.push_str(&format!(
			"\t{:width$} {:type_width$} {}\n",
			field.go_name,
			field.type_name,
			json_tag(&field.tag, field.optional),
		));
	}

	out.push_str("}\n\n");
	out
}

/// A named string type with constants, for a string-enum schema. The
/// constants share the package namespace, so they are bound in `scope`.
fn enum_declaration(schema: &Value, name: &str, scope: &mut Scope) -> String {
	let values: Vec<&str> = match analyze(schema).shape {
		Shape::String {
			values: Some(values),
			..
		} => values,
		_ => Vec::new(),
	};

	let mut out = String::new();

	if let Some(docs) = description(schema) {
		for line in docs.lines() {
			out.push_str(&format!("// {line}\n"));
		}
	}

	out.push_str(&format!("type {name} string\n\nconst (\n"));
	// Values differing only in case still get distinct constant names.
	let constants: Vec<(String, String)> = values
		.iter()
		.map(|value| {
			(
				scope.bind(&format!("{name}{}", type_name(value))),
				(*value).to_owned(),
			)
		})
		.collect();
	let width = constants
		.iter()
		.map(|(constant, _)| constant.len())
		.max()
		.unwrap_or_default();

	for (constant, value) in &constants {
		out.push_str(&format!(
			"\t{:width$} {name} = {}\n",
			constant,
			go_string(value)
		));
	}

	out.push_str(")\n\n");
	out
}

fn models_file(context: &Context<'_>, types: &mut GoTypes) -> String {
	let mut declarations = String::new();
	let mut aliases = String::new();

	for (schema_name, schema) in &context.spec.schemas {
		let name = types.components[schema_name].clone();
		let analyzed = analyze(schema);

		match (&analyzed.shape, analyzed.nullable) {
			(Shape::Object(object), false) if !object.properties.is_empty() => {
				declarations.push_str(&struct_declaration(
					&name,
					description(schema),
					object,
					types,
					context.spec,
					&name,
				));
			}

			(
				Shape::String {
					values: Some(_), ..
				},
				_,
			) => {
				declarations.push_str(&enum_declaration(schema, &name, &mut types.scope));
			}
			_ => {
				let rendered = types.render(schema, &name, context.spec);
				let rendered = if analyzed.nullable && needs_pointer(&rendered) {
					format!("*{rendered}")
				} else {
					rendered
				};
				if let Some(docs) = description(schema) {
					for line in docs.lines() {
						aliases.push_str(&format!("// {line}\n"));
					}
				}

				aliases.push_str(&format!("type {name} = {rendered}\n\n"));
			}
		}
	}

	let mut out = context.banner("//");
	out.push_str(&format!(
		"// Code generated by leptoskit; DO NOT EDIT.\n\npackage {package}\n\n// Types for the \
		 {title} {version} RPC.\n\n",
		package = go_package_name(&context.options.package_name),
		title = crate::naming::comment_text(&context.spec.title),
		version = crate::naming::comment_text(&context.spec.version)
	));

	for synthesized in &types.synthesized {
		out.push_str(synthesized);
	}

	out.push_str(&declarations);
	out.push_str(&aliases);
	out
}

/// The signature and body payload builder for one method.
struct Method {
	parameters: Vec<String>,
	encoding: &'static str,
	payload: String,
	returns: String,
}

/// The method name of every callable function, in order, clear of the
/// fields of `Client` and of each other.
fn method_names(context: &Context<'_>) -> Vec<String> {
	let mut scope = Scope::new(GO_RESERVED_METHODS);
	context
		.functions()
		.map(|function| scope.bind(&type_name(&function.name)))
		.collect()
}

/// The parameter of every argument, in order, clear of the names the method
/// body uses.
fn parameter_names(function: &ApiFunction) -> Vec<String> {
	let mut scope = Scope::new(GO_RESERVED_PARAMETERS);
	function
		.params
		.iter()
		.map(|param| scope.bind(&go_identifier(&param.name)))
		.collect()
}

fn method(function: &ApiFunction, types: &mut GoTypes, spec: &ApiSpec) -> Method {
	let hint = type_name(&function.name);
	let mut parameters = vec!["ctx context.Context".to_owned()];
	let mut fields: Vec<Field> = Vec::new();
	let identifiers = parameter_names(function);
	let mut field_names = Scope::new(&[]);

	for (param, identifier) in function.params.iter().zip(&identifiers) {
		let param_type = types.render(
			&param.schema,
			&format!("{hint}{}", type_name(&param.name)),
			spec,
		);
		parameters.push(format!("{identifier} {param_type}"));
		let optional = !param.is_required();
		fields.push(Field {
			go_name: field_names.bind(&type_name(&param.name)),
			type_name: param_type,
			tag: param.name.clone(),
			docs: Vec::new(),
			optional,
		});
	}

	let width = fields
		.iter()
		.map(|field| field.go_name.len())
		.max()
		.unwrap_or_default();
	let type_width = fields
		.iter()
		.map(|field| field.type_name.len())
		.max()
		.unwrap_or_default();
	let payload = if fields.is_empty() {
		"nil".to_owned()
	} else {
		let mut struct_text = "struct {\n".to_owned();

		for field in &fields {
			struct_text.push_str(&format!(
				"\t{:width$} {:type_width$} {}\n",
				field.go_name,
				field.type_name,
				json_tag(&field.tag, field.optional),
			));
		}

		struct_text.push_str("\t}{");
		let initializers: Vec<String> = fields
			.iter()
			.zip(&identifiers)
			.map(|(field, identifier)| format!("{}: {identifier}", field.go_name))
			.collect();
		format!("{}{}}}", struct_text, initializers.join(", "))
	};

	let output = types.render(&function.output_schema, &format!("{hint}Result"), spec);
	let returns = if output == "struct{}" {
		"error".to_owned()
	} else {
		format!("({output}, error)")
	};

	Method {
		parameters,

		encoding: match function.input {
			HttpInput::Query => "query",
			HttpInput::Form => "form",
			HttpInput::Json | HttpInput::Other { .. } => "json",
		},

		payload,
		returns,
	}
}

fn client_file(
	context: &Context<'_>,
	package: &str,
	types: &mut GoTypes,
	names: &[String],
) -> String {
	let mut methods = String::new();

	for (function, name) in context.functions().zip(names) {
		let method = method(function, types, context.spec);
		let docs = method_docs(name, function);

		for line in &docs {
			methods.push_str(&format!("// {line}\n"));
		}

		let decode = decode_call(&method, function);
		methods.push_str(&format!(
			"func (c *Client) {name}({}) {} {{\n{}}}\n\n",
			method.parameters.join(", "),
			method.returns,
			decode
		));
	}

	let mut out = context.banner("//");
	out.push_str(&format!(
		"// Code generated by leptoskit; DO NOT EDIT.\n\npackage {package}\n\nimport \
		 \"bytes\"\nimport \"context\"\nimport \"encoding/json\"\nimport \"fmt\"\nimport \
		 \"io\"\nimport \"net/http\"\nimport \"net/url\"\nimport \"strings\"\n\n// \
		 DefaultServerURL is the server origin baked in at generation time.\nconst \
		 DefaultServerURL = {server_url}\n\n{runtime}\nclass_placeholder",
		package = package,
		runtime = RUNTIME,
		server_url = go_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	));
	out = out.replace("class_placeholder", &methods);
	out
}

/// A method's docs: its name, parameter docs, idempotency, and deprecation.
fn method_docs(name: &str, function: &ApiFunction) -> Vec<String> {
	let mut docs: Vec<String> = function
		.docs
		.lines()
		.map(str::trim)
		.map(str::to_owned)
		.collect();

	if docs.is_empty() {
		docs.push(name.to_owned());
	} else {
		docs[0] = format!("{name} {}", docs[0].to_lowercase());
	}

	let documented: Vec<(&ApiParam, String)> = function
		.params
		.iter()
		.zip(parameter_names(function))
		.filter(|(param, _)| !param.docs.is_empty())
		.collect();

	if !documented.is_empty() {
		docs.push(String::new());

		for (param, identifier) in documented {
			docs.push(format!("{identifier} is {}", param.docs.trim()));
		}
	}

	if function.kind.is_idempotent_with_key() {
		docs.push(String::new());
		docs.push(
			"Pass a context from WithIdempotencyKey to make a retried call run once.".to_owned(),
		);
	}

	if function.deprecated {
		docs.push(String::new());
		docs.push("Deprecated: this function is going away.".to_owned());
	}

	docs
}

/// The call a method makes: the shared helper for a plain error, or a
/// declared result.
fn decode_call(method: &Method, function: &ApiFunction) -> String {
	if method.returns == "error" {
		return format!(
			"\treturn c.call(ctx, {}, {}, \"{}\", {})\n",
			go_string(&function.method),
			go_string(&function.path),
			method.encoding,
			method.payload
		);
	}

	// A declared zero works for every result type, where a composite literal
	// would not compile for named strings or numbers.
	format!(
		"\tvar result {result}\n\tif err := c.callInto(ctx, {method_}, {path}, \"{encoding}\", \
		 {payload}, &result); err != nil {{\n\t\tvar zero {result}\n\t\treturn zero, \
		 err\n\t}}\n\treturn result, nil\n",
		result = method
			.returns
			.trim_matches(|c| c == '(' || c == ')')
			.trim_end_matches(", error"),
		method_ = go_string(&function.method),
		path = go_string(&function.path),
		encoding = method.encoding,
		payload = method.payload,
	)
}

fn go_mod(context: &Context<'_>) -> String {
	format!(
		"{}module {}\n\ngo 1.22\n",
		context.banner("//"),
		context.options.package_name
	)
}

fn readme(context: &Context<'_>, package: &str, methods: &[String]) -> String {
	let example =
		context
			.functions()
			.zip(methods)
			.next()
			.map_or_else(String::new, |(function, method)| {
				let args: Vec<&str> = function.params.iter().map(|_| "/* ... */").collect();
				format!("result, err := client.{method}(ctx, {})\n", args.join(", "))
			});
	let mut skipped = String::new();

	if !context.skipped.is_empty() {
		skipped.push_str("\n## Not generated\n\n");

		for (name, reason) in &context.skipped {
			skipped.push_str(&format!("- `{name}`: {reason}\n"));
		}
	}

	format!(
		"# {name}

A typed Go client for the **{title} {version}** RPC, generated by `leptoskit rpc generate --target \
		 go`.

It depends only on the standard library, and the calls go to the same routes the app itself uses. \
		 Rename the module path in `go.mod` to where you publish it.

## Usage

```go
client := {package}.NewClient({server_url})

{example}if err != nil {{
	if fnErr := new({package}.FnError); errors.As(err, &fnErr) && fnErr.Retryable() {{
		// a network or server failure: safe to try again
	}}
	return err
}}
```

`FnError.Kind` is one of `KindNetwork`, `KindInvalid`, `KindServer`, `KindDecode`, or `KindOther`, \
		 mirroring the server's classification; `Retryable` is true only for network failures. \
		 Shapes Go cannot model yet (tagged unions) appear as `any` holding the decoded \
		 JSON.{skipped}",
		name = context.options.package_name,
		title = context.spec.title,
		version = context.spec.version,
		server_url = go_string(
			context
				.server_url
				.as_deref()
				.unwrap_or("https://example.com")
		),
	)
}

/// The transport, error type, and `serde_qs` encoder the package shares.
const RUNTIME: &str = r#"// FnErrorKind classifies a failed RPC call, mirroring leptoskit_rpc::FnError.
type FnErrorKind string

const (
	KindNetwork FnErrorKind = "network"
	KindInvalid FnErrorKind = "invalid"
	KindServer  FnErrorKind = "server"
	KindDecode  FnErrorKind = "decode"
	KindOther   FnErrorKind = "other"
)

// FnError is a failed RPC call.
type FnError struct {
	// Kind is the classification.
	Kind FnErrorKind
	// Message is the human-readable message.
	Message string
	// Status is the HTTP status code, when a response arrived.
	Status int
	// Raw is the response body, when a response arrived.
	Raw string
}

// Error implements error.
func (e *FnError) Error() string {
	return fmt.Sprintf("%s error: %s", e.Kind, e.Message)
}

// Retryable reports whether retrying the call could plausibly succeed: only
// when no answer arrived. Server errors are final, because the server reports
// a refusal such as "not found" the same way as an outage.
func (e *FnError) Retryable() bool {
	return e.Kind == KindNetwork
}

func fnErrorFromResponse(status int, body string) *FnError {
	variant, message, found := strings.Cut(body, "|")
	if !found {
		text := body
		if text == "" {
			text = fmt.Sprintf("HTTP %d", status)
		}
		return &FnError{Kind: KindOther, Message: text, Status: status, Raw: body}
	}
	kind := KindOther
	switch variant {
	case "Request":
		kind = KindNetwork
	case "Args", "MissingArg":
		kind = KindInvalid
	case "ServerError", "MiddlewareError", "WrappedServerFn":
		kind = KindServer
	case "Deserialization", "Serialization":
		kind = KindDecode
	}
	return &FnError{Kind: kind, Message: message, Status: status, Raw: body}
}

// Client is a typed client for the RPC.
type Client struct {
	// ServerURL is the origin the RPC is served from, without a trailing slash.
	ServerURL string
	// HTTPClient sends the requests; http.DefaultClient when nil.
	HTTPClient *http.Client
}

// NewClient returns a client for serverURL, an origin such as
// https://api.example.com. An empty string means DefaultServerURL.
func NewClient(serverURL string) *Client {
	if serverURL == "" {
		serverURL = DefaultServerURL
	}
	return &Client{ServerURL: strings.TrimRight(serverURL, "/")}
}

func (c *Client) httpClient() *http.Client {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return http.DefaultClient
}

// call runs one RPC and returns the raw JSON result, for functions that
// return nothing.
func (c *Client) call(ctx context.Context, method string, path string, encoding string, payload any) error {
	_, err := c.exchange(ctx, method, path, encoding, payload)
	return err
}

// callInto runs one RPC and decodes the JSON result into result.
func (c *Client) callInto(ctx context.Context, method string, path string, encoding string, payload any, result any) error {
	body, err := c.exchange(ctx, method, path, encoding, payload)
	if err != nil {
		return err
	}
	if len(body) == 0 || string(body) == "null" {
		return nil
	}
	if err := json.Unmarshal(body, result); err != nil {
		return &FnError{Kind: KindDecode, Message: fmt.Sprintf("the response was not JSON: %v", err), Raw: string(body)}
	}
	return nil
}

// idempotencyKeyContext is the context key WithIdempotencyKey stores under.
type idempotencyKeyContext struct{}

// WithIdempotencyKey returns a context that sends key as the Idempotency-Key
// of the mutation called with it. Calling a mutation again with the same key
// and arguments returns the first call's result instead of running it twice,
// so a call whose answer was lost is safe to retry. Use any unique string,
// such as a UUID.
func WithIdempotencyKey(ctx context.Context, key string) context.Context {
	return context.WithValue(ctx, idempotencyKeyContext{}, key)
}

func (c *Client) exchange(ctx context.Context, method string, path string, encoding string, payload any) ([]byte, error) {
	target := c.ServerURL + path
	request, err := http.NewRequestWithContext(ctx, method, target, nil)
	if err != nil {
		return nil, &FnError{Kind: KindOther, Message: err.Error()}
	}
	request.Header.Set("Accept", "application/json")
	if key, ok := ctx.Value(idempotencyKeyContext{}).(string); ok && key != "" {
		request.Header.Set("Idempotency-Key", key)
	}
	switch encoding {
	case "query":
		values, err := toQueryValues(payload)
		if err != nil {
			return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
		}
		// RawQuery carries no leading `?` of its own.
		if encoded := values.Encode(); encoded != "" {
			if request.URL.RawQuery != "" {
				request.URL.RawQuery += "&"
			}
			request.URL.RawQuery += encoded
		}
	case "form":
		values, err := toQueryValues(payload)
		if err != nil {
			return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
		}
		form := values.Encode()
		request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		if form != "" {
			request.Body = io.NopCloser(strings.NewReader(form))
			request.ContentLength = int64(len(form))
		}
	default:
		// Absent optional arguments are omitted, as serde_qs omits them: the
		// server fills defaulted arguments in, and Option arguments read a
		// missing key as None. The fields stay raw JSON, so numbers keep every
		// digit.
		if payload != nil {
			encoded, err := json.Marshal(payload)
			if err != nil {
				return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
			}
			var present map[string]json.RawMessage
			if err := json.Unmarshal(encoded, &present); err == nil {
				for key, item := range present {
					if string(item) == "null" {
						delete(present, key)
					}
				}
				if encoded, err = json.Marshal(present); err != nil {
					return nil, &FnError{Kind: KindInvalid, Message: err.Error()}
				}
			}
			request.Body = io.NopCloser(bytes.NewReader(encoded))
			request.ContentLength = int64(len(encoded))
			request.Header.Set("Content-Type", "application/json")
		}
	}
	response, err := c.httpClient().Do(request)
	if err != nil {
		return nil, &FnError{Kind: KindNetwork, Message: fmt.Sprintf("could not reach %s: %v", target, err)}
	}
	defer response.Body.Close()
	text, err := io.ReadAll(response.Body)
	if err != nil {
		return nil, &FnError{Kind: KindNetwork, Message: err.Error()}
	}
	if response.StatusCode >= 400 {
		return nil, fnErrorFromResponse(response.StatusCode, string(text))
	}
	return text, nil
}

// toQueryValues encodes a value the way serde_qs does: nested keys in
// brackets, arrays by index, nil omitted. Numbers are decoded as json.Number,
// so they are written exactly as encoding/json wrote them: 1234567, not
// 1.234567e+06, and integers above 2^53 keep every digit.
func toQueryValues(value any) (url.Values, error) {
	values := url.Values{}
	var appendValue func(key string, item any)
	appendValue = func(key string, item any) {
		switch typed := item.(type) {
		case nil:
		case bool:
			values.Set(key, fmt.Sprintf("%t", typed))
		case string:
			values.Set(key, typed)
		case json.Number:
			values.Set(key, typed.String())
		case map[string]any:
			for field, nested := range typed {
				appendValue(fmt.Sprintf("%s[%s]", key, field), nested)
			}
		case []any:
			for index, nested := range typed {
				appendValue(fmt.Sprintf("%s[%d]", key, index), nested)
			}
		}
	}
	encoded, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	decoder := json.NewDecoder(bytes.NewReader(encoded))
	decoder.UseNumber()
	// A nil payload encodes as null, which decodes to an empty tree.
	var tree map[string]any
	if err := decoder.Decode(&tree); err != nil {
		return nil, err
	}
	for key, item := range tree {
		appendValue(key, item)
	}
	return values, nil
}
"#;
