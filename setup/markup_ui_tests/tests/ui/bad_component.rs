use leptoskit::prelude::*;

#[component]
async fn greeting((first, last): (String, String), #[prop(maybe)] loud: bool) {}

fn main() {}
