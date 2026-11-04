function assert(value, message = "assertion failed") {
	if (!value) throw new Error(message);
}

function environment(reducedMotion) {
	const motion = new EventTarget();
	motion.matches = reducedMotion;
	globalThis.matchMedia = () => motion;
	globalThis.window = new EventTarget();
	globalThis.document = new EventTarget();
	document.hidden = false;
	const animations = [];
	const element = () => ({
		animate() {
			const calls = [];
			const animation = {
				finished: new Promise(() => {}),
				pause: () => calls.push("pause"),
				play: () => calls.push("play"),
				cancel: () => calls.push("cancel"),
				calls,
			};
			animations.push(animation);
			return animation;
		},
	});
	const sections = [element(), element()];
	document.querySelector = element;
	document.querySelectorAll = () => sections;
	document.getElementById = () => null;
	const observers = [];
	globalThis.IntersectionObserver = class {
		constructor(callback) {
			this.callback = callback;
			this.observed = new Set();
			observers.push(this);
		}
		observe(target) {
			this.observed.add(target);
		}
		unobserve(target) {
			this.observed.delete(target);
		}
		disconnect() {
			this.observed.clear();
		}
	};
	window.IntersectionObserver = globalThis.IntersectionObserver;
	return { motion, animations, sections, observers };
}

Deno.test("reduced motion suppresses entrances and preference changes cancel active motion", async () => {
	const env = environment(true);
	await import("../web/landing.mjs?reduced");
	assert(env.animations.length === 0);
	assert(env.observers[0].observed.size === 0);
	env.motion.matches = false;
	env.motion.dispatchEvent(new Event("change"));
	assert(env.observers[0].observed.size === 2);
	env.observers[0].callback([{
		target: env.sections[0],
		isIntersecting: true,
	}]);
	assert(env.animations.length === 1);
	env.motion.matches = true;
	env.motion.dispatchEvent(new Event("change"));
	assert(env.animations[0].calls.at(-1) === "cancel");
	assert(env.observers[0].observed.size === 0);
});

Deno.test("tab visibility pauses motion and page navigation releases it", async () => {
	const env = environment(false);
	await import("../web/landing.mjs?visible");
	assert(env.animations.length === 2);
	document.hidden = true;
	document.dispatchEvent(new Event("visibilitychange"));
	assert(
		env.animations.every((animation) => animation.calls.at(-1) === "pause"),
	);
	document.hidden = false;
	document.dispatchEvent(new Event("visibilitychange"));
	assert(
		env.animations.every((animation) => animation.calls.at(-1) === "play"),
	);
	window.dispatchEvent(new Event("pagehide"));
	assert(
		env.animations.every((animation) => animation.calls.at(-1) === "cancel"),
	);
	assert(env.observers[0].observed.size === 0);
	window.dispatchEvent(new Event("pageshow"));
	assert(env.observers[0].observed.size === 2);
});
