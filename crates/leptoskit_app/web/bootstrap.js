import init from "/pkg/leptoskit_app.js";

try {
	await init({ module_or_path: "/pkg/leptoskit_app_bg.wasm" });
} catch {
	document.body.textContent =
		"LeptosKit could not load. Refresh or contact your service operator.";
}
