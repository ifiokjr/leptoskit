// Progressive, decorative motion. The page never hides content waiting for JS.
const preference = matchMedia("(prefers-reduced-motion: reduce)");
const active = new Set();
const seen = new WeakSet();

function animate(element, frames, options = {}) {
	if (!element || preference.matches || !element.animate || document.hidden) {
		return;
	}
	const animation = element.animate(frames, {
		duration: 600,
		easing: "cubic-bezier(.2,.7,.2,1)",
		...options,
	});
	active.add(animation);
	animation.finished.catch(() => {}).finally(() => active.delete(animation));
}

animate(document.querySelector("#art-mint"), [
	{ opacity: 0, transform: "translate(18px, -65px) rotate(-22deg)" },
	{ opacity: 1, transform: "translate(0, 0) rotate(-12deg)" },
], { duration: 850 });
animate(document.querySelector("#hero-copy"), [
	{ opacity: 0, transform: "translateY(15px)" },
	{ opacity: 1, transform: "translateY(0)" },
]);

const sections = document.querySelectorAll(
	"#studio-intro, #workflow-intro, #capabilities-intro, #cloud-intro, #start",
);
const observer = "IntersectionObserver" in window
	? new IntersectionObserver((entries) => {
		for (const entry of entries) {
			if (!entry.isIntersecting || seen.has(entry.target)) continue;
			seen.add(entry.target);
			observer.unobserve(entry.target);
			animate(entry.target, [
				{ opacity: 0, transform: "translateY(20px)" },
				{ opacity: 1, transform: "translateY(0)" },
			]);
		}
	}, { threshold: .12 })
	: null;

function observe() {
	observer?.disconnect();
	if (!preference.matches) {
		for (const section of sections) {
			if (!seen.has(section)) observer?.observe(section);
		}
	}
}
observe();
preference.addEventListener("change", () => {
	for (const animation of active) animation.cancel();
	active.clear();
	observe();
});
document.addEventListener("visibilitychange", () => {
	for (const animation of active) {
		if (document.hidden) animation.pause();
		else if (!preference.matches) animation.play();
	}
});
window.addEventListener("pagehide", () => {
	observer?.disconnect();
	for (const animation of active) animation.cancel();
	active.clear();
});
window.addEventListener("pageshow", observe);

// Load the real LeptosKit Wasm example only as it comes near the viewport.
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
