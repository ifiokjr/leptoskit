// Players for the examples' web builds, which `<Play>` frames on a page.
//
// Each frame stays empty until its player comes near the viewport or takes
// focus, then loads the example's build from `play/<name>/` under the book.
// The app is laid out at its own size and scaled to the column, so loading it
// never moves the page. The frame is sandboxed; `play-frame.js` runs inside
// it and reports over `postMessage` when the app starts or fails and whether
// it can go back, and goes back when the toolbar asks.

(() => {
	"use strict";

	const players = document.querySelectorAll("[data-lk-play]");
	if (players.length === 0) return;

	const root = document.documentElement.dataset.root || "";
	// How long a frame may take to report after its page loads before the
	// player decides the page is not a played build, such as on `docs:serve`.
	const HELLO_GRACE = 1500;
	// How long a build may take to start before the player says so.
	const START_TIMEOUT = 45_000;

	/** @type {Map<Element, ReturnType<typeof player>>} */
	const byFigure = new Map();
	/** @type {Map<Window, ReturnType<typeof player>>} */
	const byFrame = new Map();

	const nearby = "IntersectionObserver" in window
		? new IntersectionObserver((entries) => {
			for (const entry of entries) {
				if (!entry.isIntersecting) continue;
				nearby.unobserve(entry.target);
				byFigure.get(entry.target)?.load();
			}
		}, { rootMargin: "300px 0px" })
		: null;

	const fit = new ResizeObserver((entries) => {
		for (const { target } of entries) {
			const figure = target.closest("[data-lk-play]");
			const scale = target.clientWidth / Number(figure.dataset.width);
			target.style.setProperty("--lk-play-scale", String(scale));
		}
	});

	/** @param {HTMLElement} figure */
	function player(figure) {
		const frame = figure.querySelector("iframe");
		const stage = figure.querySelector(".lk-play-stage");
		const status = figure.querySelector("[data-lk-play-status]");
		const label = figure.querySelector("[data-lk-play-label]");
		const back = figure.querySelector("[data-lk-play-back]");
		const reload = figure.querySelector("[data-lk-play-reload]");
		const open = figure.querySelector("[data-lk-play-open]");
		const native = figure.querySelector("img.lk-play-native[data-src]");
		const views = figure.querySelectorAll("[data-lk-play-view]");
		const title = figure.querySelector("figcaption > strong")?.textContent ??
			"The example";
		const source = new URL(root + frame.dataset.src, location.href).href;
		let hello = false;
		let timer = 0;
		// Set once the native render fails to load, such as when the host's
		// policy blocks images from elsewhere.
		const unavailable = `The AppKit render of ${title} could not load.`;
		let nativeFailed = false;

		// Shows the web build or the native render in the same frame.
		const select = (view) => {
			figure.dataset.view = view;
			for (const button of views) {
				button.setAttribute(
					"aria-pressed",
					String(button.dataset.lkPlayView === view),
				);
			}
			if (view === "native" && native && !native.src) {
				native.src = native.dataset.src;
			}
			if (native) native.hidden = view !== "native";
			if (view === "native" && nativeFailed) status.textContent = unavailable;
			else if (status.textContent === unavailable) status.textContent = "";
			label.textContent = view === "native"
				? "AppKit views, drawn on macOS"
				: "Web build, in your browser";
		};

		const show = (state, message = "") => {
			figure.dataset.state = state;
			status.textContent = message;
			clearTimeout(timer);
		};

		const start = (navigate) => {
			hello = false;
			back.disabled = true;
			show("loading", `Loading ${title}…`);
			navigate();
			timer = setTimeout(() => {
				show(
					"failed",
					`${title} is taking too long to start. Reload to try again.`,
				);
			}, START_TIMEOUT);
		};

		const api = {
			load() {
				if (figure.dataset.state !== "idle") return;
				start(() => frame.src = source);
			},
			/** @param {{ type: string, canGoBack?: boolean, message?: string }} data */
			receive(data) {
				switch (data.type) {
					case "hello":
						hello = true;
						break;
					case "ready":
						if (figure.dataset.state === "loading") show("ready");
						break;
					case "history":
						back.disabled = !data.canGoBack;
						break;
					case "error":
						show(
							"failed",
							`${title} stopped with an error: ${data.message}. Reload to start it again.`,
						);
						break;
				}
			},
		};

		native?.addEventListener("error", () => {
			nativeFailed = true;
			native.dataset.failed = "";
			if (figure.dataset.view === "native") status.textContent = unavailable;
		});

		frame.addEventListener("load", () => {
			if (figure.dataset.state !== "loading") return;
			setTimeout(() => {
				if (!hello && figure.dataset.state === "loading") {
					show(
						"missing",
						native
							? `${title} isn't built into this copy of the docs. AppKit shows its native render.`
							: `${title} isn't built into this copy of the docs.`,
					);
					if (native) select("native");
				}
			}, HELLO_GRACE);
		});

		back.addEventListener("click", () => {
			frame.contentWindow.postMessage({ lk: "play", type: "back" }, "*");
		});

		// Start again from the example's first screen. Replacing the frame's
		// location adds no entry to the page's history.
		reload.addEventListener("click", () => {
			select("web");
			start(() => frame.contentWindow.location.replace(source));
		});

		for (const button of views) {
			button.disabled = false;
			button.addEventListener("click", () => {
				select(button.dataset.lkPlayView);
				if (button.dataset.lkPlayView === "web") api.load();
			});
		}

		open.href = source;
		reload.disabled = false;
		figure.dataset.state = "idle";
		figure.dataset.view = "web";
		// Keyboard focus reaches the frame before it scrolls into view.
		figure.addEventListener("focusin", api.load);
		fit.observe(stage);
		return api;
	}

	for (const figure of players) {
		const api = player(figure);
		byFigure.set(figure, api);
		byFrame.set(figure.querySelector("iframe").contentWindow, api);
		if (nearby) nearby.observe(figure);
		else api.load();
	}

	addEventListener("message", (event) => {
		const data = event.data;
		if (!data || data.lk !== "play" || typeof data.type !== "string") return;
		// A frame keeps its origin only when its example routes; either way
		// its window is the one this player made.
		if (event.origin !== "null" && event.origin !== location.origin) return;
		byFrame.get(event.source)?.receive(data);
	});
})();
