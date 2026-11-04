import init from "/pkg/leptoskit_app.js";

try {
	// The pinned runtime stays on our origin. A decorative asset failure must
	// not prevent the management app from starting.
	if (!globalThis.rive) {
		await new Promise((resolve) => {
			const timeout = setTimeout(resolve, 3000);
			const finish = () => {
				clearTimeout(timeout);
				resolve();
			};
			const script = document.createElement("script");
			script.src = "/rive/rive.js";
			script.onload = finish;
			script.onerror = finish;
			document.head.append(script);
		});
	}
	await init({ module_or_path: "/pkg/leptoskit_app_bg.wasm" });
} catch {
	const note = document.getElementById("preview-note");
	if (note) {
		note.textContent =
			"The interactive example could not load. The guides and the rest of the site are still available.";
	} else {
		document.body.textContent =
			"LeptosKit could not load. Refresh or contact your service operator.";
	}
}
