function assert(value, message = "assertion failed") {
	if (!value) throw new Error(message);
}

// The module's only job is deferring the live example: it must not animate
// anything itself, and it must import the client once, when the example
// nears the viewport or receives focus.
function environment({ observer = true } = {}) {
	globalThis.window = new EventTarget();
	globalThis.document = new EventTarget();
	const demo = new EventTarget();
	document.getElementById = (id) => (id === "live-demo" ? demo : null);
	const observers = [];
	if (observer) {
		globalThis.IntersectionObserver = class {
			constructor(callback, options) {
				this.callback = callback;
				this.options = options;
				this.observed = new Set();
				observers.push(this);
			}
			observe(target) {
				this.observed.add(target);
			}
			disconnect() {
				this.observed.clear();
			}
		};
		window.IntersectionObserver = globalThis.IntersectionObserver;
	} else {
		delete globalThis.IntersectionObserver;
	}
	return { demo, observers };
}

Deno.test("the client loads once the example nears the viewport", async () => {
	const env = environment();
	await import("../web/landing.mjs?near");
	assert(env.observers.length === 1, "one observer watches the example");
	const [loader] = env.observers;
	assert(loader.observed.has(env.demo));
	assert(loader.options.rootMargin === "250px");
	loader.callback([{ target: env.demo, isIntersecting: false }]);
	assert(loader.observed.size === 1, "a distant example keeps waiting");
	loader.callback([{ target: env.demo, isIntersecting: true }]);
	assert(loader.observed.size === 0, "the observer lets go once it loads");
});

Deno.test("keyboard focus loads the example before it scrolls into view", async () => {
	const env = environment();
	await import("../web/landing.mjs?focus");
	env.demo.dispatchEvent(new Event("focusin"));
	// Loading twice would be harmless but wasteful: the import is shared.
	env.observers[0].callback([{ target: env.demo, isIntersecting: true }]);
	assert(env.observers[0].observed.size === 0);
});

Deno.test("the module animates nothing itself", async () => {
	const env = environment();
	const animated = [];
	globalThis.requestAnimationFrame = () =>
		animated.push("requestAnimationFrame");
	window.matchMedia = () => animated.push("matchMedia");
	env.demo.animate = () => animated.push("animate");
	await import("../web/landing.mjs?still");
	env.observers[0].callback([{ target: env.demo, isIntersecting: true }]);
	env.demo.dispatchEvent(new Event("focusin"));
	delete globalThis.requestAnimationFrame;
	assert(
		animated.length === 0,
		`landing.mjs animated with ${animated.join(", ")}`,
	);
});
