// Everything the client needs starts downloading at once: its Wasm module,
// its JavaScript glue, and the Rive runtime. A static import of the glue
// would hold this module back until the glue and its own imports arrived,
// one round trip after another, before any download below could begin.
//
// The runtime asks for priority over the module: where the connection
// honours it, the runtime arrives about as soon as it did alone, well within
// its timeout, and the module takes the bandwidth left over. A failed
// download surfaces where it is awaited below, so mark each handled until
// then.
const client = fetch("/pkg/leptoskit_app_bg.wasm", { priority: "low" });
client.catch(() => {});
const glue = import("/pkg/leptoskit_app.js");
glue.catch(() => {});

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
			script.fetchPriority = "high";
			script.onload = finish;
			script.onerror = finish;
			document.head.append(script);
		});
	}
	const { default: init } = await glue;
	await init({ module_or_path: client });
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
