#[cfg(not(feature = "probe"))]
compile_error!("build.features did not reach cargo");

const PLATFORM: &str = env!("LEPTOSKIT_PLUGIN_PLATFORM");

fn main() {
	println!("{PLATFORM}");
}
