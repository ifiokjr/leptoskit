//! Export actual `LeptosKit`-rendered screens without starting a server.
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;

use base64::Engine as _;
use leptos::prelude::RenderHtml;
use leptoskit_deploy_console::model::Intent;
use leptoskit_deploy_console::model::Model;
use leptoskit_deploy_console::model::SchemaChange;
use leptoskit_deploy_console::model::Tab;
use leptoskit_deploy_console::model::Target;
use leptoskit_deploy_console::view;

fn render(model: &Model, sheets: &mut BTreeSet<String>) -> String {
	let mut html = leptoskit::web::render(view::screen(model))
		.to_html()
		.replace("<!>", "");
	while let Some(start) = html.find("<style") {
		let end = start
			+ html[start..]
				.find("</style>")
				.expect("renderer closes styles")
			+ 8;
		sheets.insert(html[start..end].to_owned());
		html.replace_range(start..end, "");
	}
	html
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let path = std::env::args()
		.nth(1)
		.unwrap_or_else(|| "target/deploy-console.html".into());
	let mut templates = String::new();
	let mut sheets = BTreeSet::new();
	for (t, target) in [Target::SingleVm, Target::RedundantVm, Target::Kubernetes]
		.into_iter()
		.enumerate()
	{
		for (s, schema) in [
			SchemaChange::Unchanged,
			SchemaChange::ExpandUnverified,
			SchemaChange::Reverse,
		]
		.into_iter()
		.enumerate()
		{
			for (i, intent) in [None, Some(Intent::Deploy), Some(Intent::Rollback)]
				.into_iter()
				.enumerate()
			{
				let mut model = Model::new(target, Tab::Deployments, schema);
				if let Some(intent) = intent {
					model.preview(intent);
				}
				write!(
					templates,
					"<template id=\"d-{t}-{s}-{i}\">{}</template>",
					render(&model, &mut sheets)
				)?;
			}
		}
		let model = Model::new(target, Tab::Infrastructure, SchemaChange::Unchanged);
		write!(
			templates,
			"<template id=\"i-{t}\">{}</template>",
			render(&model, &mut sheets)
		)?;
	}
	for (id, self_hosted) in [(0, false), (1, true)] {
		let mut model = Model::new(Target::SingleVm, Tab::Agents, SchemaChange::Unchanged);
		model.self_hosted = self_hosted;
		write!(
			templates,
			"<template id=\"a-{id}\">{}</template>",
			render(&model, &mut sheets)
		)?;
	}
	let mut initial_sheets = BTreeSet::new();
	let initial = render(&Model::default(), &mut initial_sheets);
	let styles = initial_sheets.into_iter().collect::<String>();
	let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	compressed.write_all(sheets.into_iter().collect::<String>().as_bytes())?;
	compressed.write_all(templates.as_bytes())?;
	let packed = base64::engine::general_purpose::STANDARD.encode(compressed.finish()?);
	let html = format!(
		r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>LeptosKit deployment console · prototype</title>
{styles}<style>body{{margin:0;font-family:var(--font-sans,system-ui)}} html,body,#app{{background:transparent}} #app{{color:var(--foreground,inherit)}} #app button{{cursor:pointer}} #app p{{overflow-wrap:anywhere}} </style></head><body><main id="app">{initial}</main><div id="variants" hidden></div>
<script>
let tab='d',target=0,schema=0,intent=0,hosting=0;
const ready=(async()=>{{
 const bytes=Uint8Array.from(atob('{packed}'),c=>c.charCodeAt(0));
 const stream=new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip'));
 document.getElementById('variants').innerHTML=await new Response(stream).text();
}})();
document.addEventListener('click',async event=>{{
 const node=event.target.closest('[data-testid]'); if(!node)return;
 const action=node.getAttribute('data-testid');
 if(action==='tab-deployments')tab='d'; else if(action==='tab-infrastructure')tab='i'; else if(action==='tab-agents')tab='a';
 else if(action==='target-small'){{target=0;intent=0}} else if(action==='target-multiple'){{target=1;intent=0}} else if(action==='target-kubernetes'){{target=2;intent=0}}
 else if(action==='schema-same'){{schema=0;intent=0}} else if(action==='schema-expand'){{schema=1;intent=0}} else if(action==='schema-reverse'){{schema=2;intent=0}}
 else if(action==='preview-deploy')intent=1; else if(action==='preview-rollback')intent=2;
 else if(action==='hosting-managed')hosting=0; else if(action==='hosting-self')hosting=1; else return;
 await ready;
 const key=tab==='d'?`d-${{target}}-${{schema}}-${{intent}}`:tab==='i'?`i-${{target}}`:`a-${{hosting}}`;
 document.getElementById('app').replaceChildren(document.getElementById(key).content.cloneNode(true));
}});
</script></body></html>"#
	);
	if let Some(parent) = std::path::Path::new(&path).parent() {
		std::fs::create_dir_all(parent)?;
	}
	std::fs::write(&path, html)?;
	println!("{path}");
	Ok(())
}
