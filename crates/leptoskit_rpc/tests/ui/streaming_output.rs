use leptos::prelude::*;
use leptos::server_fn::codec::StreamingText;
use leptos::server_fn::codec::TextStream;
use leptoskit_rpc::api;

#[api]
#[server(output = StreamingText)]
pub async fn tail() -> Result<TextStream, ServerFnError> {
	unimplemented!()
}

fn main() {}
