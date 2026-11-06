// The page's motion is CSS that leptoskit's renderer writes; this module
// only defers the live example. It loads the real LeptosKit Wasm client when
// the example comes near the viewport or receives focus, so the first paint
// never waits for it. Without JavaScript the static example stays readable.
const demo = document.getElementById("live-demo");
let client;

function loadDemo() {
	client ??= import("/bootstrap.js");
	return client;
}

if (demo) {
	if ("IntersectionObserver" in window) {
		const loader = new IntersectionObserver((entries) => {
			if (entries.some((entry) => entry.isIntersecting)) {
				loader.disconnect();
				loadDemo().catch(() => {});
			}
		}, { rootMargin: "250px" });
		loader.observe(demo);
	} else {
		loadDemo().catch(() => {});
	}
	demo.addEventListener("focusin", () => loadDemo().catch(() => {}), {
		once: true,
	});
}
