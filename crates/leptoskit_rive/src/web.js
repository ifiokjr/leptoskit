// Lifecycle stays beside Rive's render loop; Rust never participates per frame.
const runtimes = new WeakMap();
const canvases = new WeakSet();

export function configure(module, wasmUrl) {
	if (
		typeof module?.Rive !== "function" ||
		typeof module?.RuntimeLoader?.setWasmUrl !== "function" ||
		typeof module?.RuntimeLoader?.setWasmFallbackUrl !== "function"
	) {
		throw new Error("expected an official Rive runtime namespace");
	}

	const url = new URL(wasmUrl, location.href);
	if (
		url.origin !== location.origin ||
		!["https:", "http:", "blob:"].includes(url.protocol)
	) {
		throw new Error("Rive Wasm must be served from the page's origin");
	}

	const existing = runtimes.get(module);
	if (existing && existing !== url.href) {
		throw new Error(
			"Rive runtime was already configured with a different Wasm URL",
		);
	}

	if (!existing) {
		// A self-hosted primary URL otherwise falls back to a public CDN on failure.
		module.RuntimeLoader.setWasmFallbackUrl(null);
		module.RuntimeLoader.setWasmUrl(url.href);
		runtimes.set(module, url.href);
	}
}

export function mount(
	module,
	canvas,
	bytes,
	artboard,
	sceneKind,
	scene,
	autoplay,
) {
	if (!runtimes.has(module)) {
		throw new Error("configure the Rive runtime before mounting");
	}
	if (canvases.has(canvas)) {
		throw new Error("canvas already owns a Rive player");
	}
	if (!canvas.isConnected) {
		throw new Error("attach the canvas before mounting Rive");
	}
	if (
		!canvas.getAttribute("aria-label")?.trim() &&
		canvas.getAttribute("aria-hidden") !== "true"
	) {
		throw new Error("give the canvas an aria-label or mark it aria-hidden");
	}

	let rive;
	let disposed = false;
	let loaded = false;
	let failure;
	let wanted = autoplay;
	let intersecting = false;
	let hasSize = false;
	let running = false;
	let settle;
	const ready = new Promise((resolve) => {
		settle = resolve;
	});
	const motion = matchMedia("(prefers-reduced-motion: reduce)");
	const previousWidth = canvas.width;
	const previousHeight = canvas.height;
	let resize;
	let intersection;
	let timer;
	let inputs;

	function assertLive() {
		if (disposed) throw new Error(failure ?? "Rive player was disposed");
	}

	function syncPlayback() {
		if (!loaded || disposed) return;
		const active = wanted && intersecting && hasSize && !document.hidden &&
			!motion.matches;
		if (active === running) return;
		running = active;
		if (active) {
			rive.play();
			rive.startRendering();
		} else {
			rive.pause();
			rive.stopRendering();
		}
	}

	function resizeCanvas() {
		if (disposed) return;
		const bounds = canvas.getBoundingClientRect();
		hasSize = bounds.width > 0 && bounds.height > 0;
		if (hasSize) {
			// Bound GPU allocation even for giant CSS boxes or high-DPI displays.
			const scale = Math.min(
				devicePixelRatio || 1,
				2,
				4096 / bounds.width,
				4096 / bounds.height,
				Math.sqrt(4194304 / bounds.width / bounds.height),
			);
			const width = Math.max(1, Math.floor(bounds.width * scale));
			const height = Math.max(1, Math.floor(bounds.height * scale));
			if (canvas.width !== width || canvas.height !== height) {
				canvas.width = width;
				canvas.height = height;
				if (loaded) rive.resizeToCanvas();
			}
		}
		syncPlayback();
	}

	function dispose() {
		if (disposed) return;
		disposed = true;
		clearTimeout(timer);
		resize?.disconnect();
		intersection?.disconnect();
		document.removeEventListener("visibilitychange", syncPlayback);
		motion.removeEventListener("change", syncPlayback);
		globalThis.removeEventListener("resize", resizeCanvas);
		// Cleanup owns pending imports too; the runtime checks its destroyed flag.
		try {
			rive?.cleanup();
		} finally {
			inputs?.clear();
			canvases.delete(canvas);
			canvas.width = previousWidth;
			canvas.height = previousHeight;
			settle(failure ?? "Rive player was disposed before loading");
		}
	}

	function fail(message) {
		if (disposed) return;
		failure = String(message);
		settle(failure);
		dispose();
	}

	function inputFor(name, kind) {
		assertLive();
		if (!loaded) throw new Error("wait for Rive import before setting inputs");
		if (sceneKind !== "stateMachine") {
			throw new Error("player has no selected state machine");
		}
		inputs ??= new Map(
			rive.stateMachineInputs(scene).map((input) => [input.name, input]),
		);
		const input = inputs.get(name);
		if (!input) throw new Error(`unknown Rive input: ${name}`);
		const expected = { number: 56, boolean: 59, trigger: 58 }[kind];
		if (input.type !== expected) {
			throw new Error(`Rive input type mismatch: ${name}`);
		}
		return input;
	}

	canvases.add(canvas);
	try {
		resize = new ResizeObserver(resizeCanvas);
		intersection = new IntersectionObserver((entries) => {
			if (disposed) return;
			intersecting = entries.some((entry) => entry.isIntersecting);
			syncPlayback();
		});
		resize.observe(canvas);
		intersection.observe(canvas);
		document.addEventListener("visibilitychange", syncPlayback);
		motion.addEventListener("change", syncPlayback);
		globalThis.addEventListener("resize", resizeCanvas);
		resizeCanvas();
		timer = setTimeout(
			() => fail("Rive load timed out after 15 seconds"),
			15000,
		);
		rive = new module.Rive({
			canvas,
			buffer: bytes.buffer,
			artboard: artboard ?? undefined,
			animations: sceneKind === "animation" ? scene : undefined,
			stateMachine: sceneKind === "stateMachine" ? scene : undefined,
			autoplay: false,
			autoBind: false,
			enableRiveAssetCDN: false,
			automaticallyHandleEvents: false,
			shouldDisableRiveListeners: true,
			// A missing embedded asset cannot silently trigger an external fetch.
			assetLoader: (_asset, content) => {
				if (content.length) return false;
				queueMicrotask(() =>
					fail("Rive asset must embed its images, fonts, and audio")
				);
				return true;
			},
			onLoadError: (event) => fail(event.data ?? "Rive import failed"),
			onLoad: () => {
				// Let upstream finish its initial draw and playback task queue first.
				queueMicrotask(() => {
					if (disposed) return;
					try {
						const names = sceneKind === "animation"
							? rive.animationNames
							: rive.stateMachineNames;
						if (scene && !names.includes(scene)) {
							throw new Error(`unknown Rive scene: ${scene}`);
						}
						loaded = true;
						clearTimeout(timer);
						rive.resizeToCanvas();
						rive.pause();
						rive.stopRendering();
						syncPlayback();
						settle(null);
					} catch (error) {
						fail(error.message);
					}
				});
			},
		});
		rive.volume = 0;
	} catch (error) {
		dispose();
		throw error;
	}

	return {
		ready,
		play() {
			assertLive();
			wanted = true;
			syncPlayback();
		},
		pause() {
			assertLive();
			wanted = false;
			syncPlayback();
		},
		setInput(name, kind, value) {
			const input = inputFor(name, kind);
			if (kind === "trigger") input.fire();
			else input.value = value;
		},
		validateInput(name, kind) {
			inputFor(name, kind);
		},
		dispose,
	};
}
