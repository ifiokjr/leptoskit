use rustc_version::Channel;
use rustc_version::version_meta;

fn main() {
	println!("cargo:rustc-check-cfg=cfg(rustc_nightly)");

	if matches!(version_meta().unwrap().channel, Channel::Nightly) {
		println!("cargo:rustc-cfg=rustc_nightly");
	}
}
