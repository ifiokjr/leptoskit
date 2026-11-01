import { configure, mount } from "../src/web.js";

function assert(value, message = "assertion failed") {
	if (!value) throw new Error(message);
}

function throws(fn, expected) {
	try {
		fn();
	} catch (error) {
		assert(error.message.includes(expected), error.message);

		return;
	}
	throw new Error(`expected failure: ${expected}`);
}

// Exercise browser event/ownership boundaries without depending on timing.
function environment() {
	globalThis.location = new URL("https://app.test/");
	globalThis.document = new EventTarget();
	document.hidden = false;
	globalThis.devicePixelRatio = 8;
	const motion = new EventTarget();
	motion.matches = false;
	globalThis.matchMedia = () => motion;
	const observers = [];
	globalThis.ResizeObserver = class {
		constructor(callback) {
			this.callback = callback;
			observers.push(this);
		}
		observe() {}
		disconnect() {
			this.disconnected = true;
		}
	};
	globalThis.IntersectionObserver = globalThis.ResizeObserver;
	const events = [];
	let instance;
	const module = {
		RuntimeLoader: {
			setWasmUrl: (url) => events.push(["wasm", url]),
			setWasmFallbackUrl: (url) => events.push(["fallback", url]),
		},
		Rive: class {
			constructor(options) {
				this.options = options;
				this.animationNames = ["Spin"];
				this.stateMachineNames = ["Machine"];
				instance = this;
			}
			play() {
				events.push("play");
			}
			pause() {
				events.push("pause");
			}
			startRendering() {
				events.push("start");
			}
			stopRendering() {
				events.push("stop");
			}
			resizeToCanvas() {
				events.push("resize");
			}
			cleanup() {
				events.push("cleanup");
			}
		},
	};
	const canvas = {
		isConnected: true,
		width: 300,
		height: 150,
		hasAttribute: () => true,
		getAttribute: () => "Triangle",
		getBoundingClientRect: () => ({ width: 128, height: 128 }),
	};
	configure(module, "/rive.wasm");
	const create = () =>
		mount(module, canvas, new Uint8Array([1]), null, "animation", "Spin", true);
	const visible = (value) => observers[1].callback([{ isIntersecting: value }]);
	const load = async () => {
		instance.options.onLoad();
		await Promise.resolve();
	};

	return {
		module,
		canvas,
		create,
		visible,
		load,
		events,
		observers,
		motion,
		instance: () => instance,
	};
}

Deno.test("runtime configuration cannot switch origin or change after initialization", () => {
	const env = environment();
	configure(env.module, "/rive.wasm");
	assert(env.events.length === 2);
	assert(env.events[0][0] === "fallback" && env.events[0][1] === null);
	throws(
		() => configure(env.module, "https://outside.test/rive.wasm"),
		"origin",
	);
	throws(
		() => configure(env.module, "data:application/wasm;base64,AA=="),
		"origin",
	);
	throws(() => configure(env.module, "/other.wasm"), "different");
	throws(() => configure({}, "/rive.wasm"), "namespace");
});

Deno.test("visibility, reduced motion, and pause jointly gate rendering", async () => {
	const env = environment();
	const player = env.create();
	await env.load();
	assert(await player.ready === null);
	assert(!env.events.includes("start"));
	env.visible(true);
	assert(env.events.at(-1) === "start");
	document.hidden = true;
	document.dispatchEvent(new Event("visibilitychange"));
	assert(env.events.at(-1) === "stop");
	document.hidden = false;
	document.dispatchEvent(new Event("visibilitychange"));
	assert(env.events.at(-1) === "start");
	env.motion.matches = true;
	env.motion.dispatchEvent(new Event("change"));
	assert(env.events.at(-1) === "stop");
	player.pause();
	env.motion.matches = false;
	env.motion.dispatchEvent(new Event("change"));
	assert(env.events.at(-1) === "stop");
	player.play();
	assert(env.events.at(-1) === "start");
	env.visible(false);
	assert(env.events.at(-1) === "stop");
	player.dispose();
	player.dispose();
	assert(env.events.filter((event) => event === "cleanup").length === 1);
	assert(env.observers.every((observer) => observer.disconnected));
	const count = env.events.length;
	document.dispatchEvent(new Event("visibilitychange"));
	env.motion.dispatchEvent(new Event("change"));
	globalThis.dispatchEvent(new Event("resize"));
	assert(env.events.length === count);
	assert(env.canvas.width === 300 && env.canvas.height === 150);
});

Deno.test("file content cannot enable CDN access, navigation, audio, or external assets", async () => {
	const env = environment();
	const player = env.create();
	const rive = env.instance();
	assert(rive.options.enableRiveAssetCDN === false);
	assert(rive.options.automaticallyHandleEvents === false);
	assert(rive.options.shouldDisableRiveListeners === true);
	assert(rive.volume === 0);
	assert(rive.options.assetLoader({}, new Uint8Array([1])) === false);
	assert(rive.options.assetLoader({}, new Uint8Array()) === true);
	assert((await player.ready).includes("embed"));
	throws(() => player.play(), "embed");
	assert(env.events.at(-1) === "cleanup");
});

Deno.test("pending imports, failed imports, and missing scenes all release ownership", async () => {
	const env = environment();
	let player = env.create();
	const pending = env.instance();
	player.dispose();
	assert((await player.ready).includes("disposed"));
	pending.options.onLoad();
	await Promise.resolve();
	assert(!env.events.includes("start"));
	player = env.create();
	env.instance().options.onLoadError({ data: "bad file" });
	assert(await player.ready === "bad file");
	player = env.create();
	env.instance().animationNames = [];
	await env.load();
	assert((await player.ready).includes("unknown Rive scene"));
	assert(env.observers.every((observer) => observer.disconnected));
});

Deno.test("mount failures and upstream cleanup errors preserve canvas ownership", async () => {
	const env = environment();
	env.canvas.isConnected = false;
	throws(env.create, "attach");
	env.canvas.isConnected = true;
	env.canvas.hasAttribute = () => false;
	env.canvas.getAttribute = () => null;
	throws(env.create, "aria-label");
	env.canvas.hasAttribute = () => true;
	env.canvas.getAttribute = () => "  ";
	throws(env.create, "aria-label");
	env.canvas.getAttribute = () => "Triangle";
	const player = env.create();
	throws(env.create, "already owns");
	env.instance().cleanup = () => {
		throw new Error("cleanup failed");
	};
	throws(() => player.dispose(), "cleanup failed");
	assert(env.observers.every((observer) => observer.disconnected));
	assert((await player.ready).includes("disposed"));
	env.create().dispose();
});

Deno.test("a load deadline cleans up and constructor failures allow another mount", async () => {
	const env = environment();
	const originalTimer = globalThis.setTimeout;
	let deadline;
	globalThis.setTimeout = (callback, milliseconds) => {
		assert(milliseconds === 15000);
		deadline = callback;
		return 0;
	};
	try {
		const player = env.create();
		deadline();
		assert((await player.ready).includes("timed out"));
		throws(() => player.pause(), "timed out");
		const constructor = env.module.Rive;
		env.module.Rive = class {
			constructor() {
				throw new Error("no renderer");
			}
		};
		throws(env.create, "no renderer");
		env.module.Rive = constructor;
		env.create().dispose();
		assert(env.observers.every((observer) => observer.disconnected));
	} finally {
		globalThis.setTimeout = originalTimer;
	}
});
