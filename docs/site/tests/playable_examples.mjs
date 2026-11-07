// The book's players, in headless Chrome. On each packaged page that plays
// examples, every `<Play>` must load its example's web build when it nears
// the viewport, the app must mount inside the player's sandboxed frame, and
// one real press, keystroke, or scroll must change what the app shows. Field
// must follow a link to another route and come back with the toolbar's back
// button, and a player must switch to its AppKit image and back. No page or
// frame may log an error.
//
// Package the site first, then point the test at the artifact (the test fails
// without one):
//
//   LEPTOSKIT_SITE=docs/dist deno test --allow-env --allow-read --allow-net \
//     --allow-run --allow-write docs/site/tests/playable_examples.mjs
//
// Input goes to the page, as a reader's would: the browser routes it into the
// frame under the pointer, through the player's scaling, whether that frame
// runs in the page's process or in one of its own. Checks run inside each
// frame through the DevTools session it is attached to.
//
// `CHROME` names the browser executable; otherwise the usual install
// locations are tried. See `browser.mjs` for how the test drives Chrome.

import {
	assert,
	launch,
	Page,
	removeProfile,
	serve,
} from "../../../crates/leptoskit_app/tests/browser.mjs";

const site = Deno.env.get("LEPTOSKIT_SITE")?.replace(/\/$/, "");

// How long an example may take to download, compile, and mount.
const MOUNT_TIMEOUT = 60_000;

// Expressions that find an element inside a played app.
const find = {
	css: (selector) => `document.querySelector(${JSON.stringify(selector)})`,
	text: (tag, text) =>
		`[...document.querySelectorAll(${JSON.stringify(tag)})]
			.find((element) => element.textContent.trim() === ${
			JSON.stringify(text)
		})`,
	// The first element that scrolls its content vertically.
	scroller: `[...document.querySelectorAll("*")].find((element) =>
		element.scrollHeight > element.clientHeight + 1 &&
		/auto|scroll/.test(getComputedStyle(element).overflowY))`,
};

/**
 * One interaction per player, by the player's id: `target` finds what the
 * reader acts on, which also shows the app has mounted, and `play` acts and
 * waits for the app to show the change. A page with a player missing here
 * fails, so a new player needs its own interaction.
 */
const PLAYS = {
	"lk-play-counter": {
		target: find.text("button", "Add one"),
		async play(player) {
			await player.press(this.target);
			await player.until(find.text("span", "13"), "the count reaching 13");
		},
	},
	"lk-play-profile": {
		target: find.text("button", "Follow"),
		async play(player) {
			await player.press(this.target);
			await player.until(
				`${find.text("button", "Following")} && ${find.text("span", "1,285")}`,
				"the profile showing it is followed",
			);
		},
	},
	"lk-play-settings": {
		target: find.css('input[aria-label="Airplane Mode"]'),
		async play(player) {
			await player.press(this.target);
			await player.until(
				find.css('input[aria-label="Airplane Mode"][data-lk-checked="true"]'),
				"Airplane Mode switching on",
			);
		},
	},
	"lk-play-photos": {
		target: find.scroller,
		play: (player) => player.scroll(find.scroller),
	},
	"lk-play-chat": {
		target: find.css('input[aria-label="Message"]'),
		async play(player) {
			await player.press(this.target);
			await player.type("See you at six");
			await player.until(
				`${this.target}?.value === "See you at six"`,
				"the composer showing the typed message",
			);
		},
	},
	"lk-play-dashboard": {
		target: find.text("button", "7 days"),
		async play(player) {
			await player.until(
				find.text("span", "$92,418"),
				"the chart starting on 30 days",
			);
			await player.press(this.target);
			await player.until(
				find.text("span", "$23,840"),
				"the chart showing the last 7 days",
			);
		},
	},
	"lk-play-component-studio": {
		target: find.css('[aria-label="Add a preview"]'),
		async play(player) {
			await player.press(this.target);
			await player.until(
				find.text("span", "13 previews created"),
				"the preview count reaching 13",
			);
		},
	},
	"lk-play-observatory": {
		target: find.css('button[aria-label="Night 3"]'),
		async play(player) {
			await player.press(this.target);
			await player.until(
				find.css('button[aria-label="Night 3"][aria-pressed="true"]'),
				"the third night being picked",
			);
		},
	},
	"lk-play-postcard": {
		target: find.text("button", "ja"),
		async play(player) {
			await player.until(
				`document.body.textContent.includes("تحيات")`,
				"the card starting in Arabic",
			);
			await player.press(this.target);
			await player.until(
				`${find.css('[aria-pressed="true"]')}?.textContent === "ja" &&
					!document.body.textContent.includes("تحيات")`,
				"the card switching to Japanese",
			);
		},
	},
	// Each of Field's tabs shows its name above the tab bar.
	"lk-play-field": {
		target: find.css('a[aria-label="Map"]'),
		async play(player, page) {
			const shown = (tab) =>
				`location.pathname === "/docs/play/field/${
					tab === "Today" ? "" : tab.toLowerCase()
				}" &&
					document.querySelector("span")?.textContent === "${tab}"`;
			const back = `#${player.id} [data-lk-play-back]`;
			await player.until(shown("Today"), "Field starting on today");
			assert(
				await page.evaluate(
					`document.querySelector(${JSON.stringify(back)}).disabled`,
				),
				"Field's back button should wait for somewhere to go back to",
			);
			await player.press(this.target);
			await player.until(shown("Map"), "Field following its link to the map");
			await page.until(
				`() => !document.querySelector(${JSON.stringify(back)}).disabled`,
				"the toolbar offering to go back",
			);
			await pressToolbar(page, back);
			await player.until(shown("Today"), "Field going back to today");
			await page.until(
				`() => document.querySelector(${JSON.stringify(back)}).disabled`,
				"the toolbar's back button resting at the first page",
			);
		},
	},
};

/**
 * Waits until `check` returns a truthy value. A frame's execution context
 * goes away while it navigates, so a failed check is retried, and the last
 * failure explains a timeout.
 */
async function waitFor(check, description, timeout = 20_000) {
	const deadline = Date.now() + timeout;
	let failure;
	for (;;) {
		try {
			const value = await check();
			if (value) return value;
		} catch (error) {
			failure = error;
		}
		if (Date.now() > deadline) {
			throw new Error(
				`timed out: ${description}${failure ? ` (${failure.message})` : ""}`,
			);
		}
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
}

/**
 * The frames of a page and the DevTools session and execution context that
 * evaluate in each. A frame in the page's process reports its context on the
 * page's session; one in a process of its own is attached as a target with
 * its own session.
 */
class Frames {
	#page;
	#contexts = new Map();

	constructor(page) {
		this.#page = page;
		page.on("Runtime.executionContextCreated", ({ context }, sessionId) => {
			const { frameId, isDefault } = context.auxData ?? {};
			if (frameId && isDefault) {
				this.#contexts.set(frameId, { sessionId, contextId: context.id });
			}
		});
		page.on(
			"Runtime.executionContextDestroyed",
			({ executionContextId }, sessionId) => {
				this.#forget((context) =>
					context.sessionId === sessionId &&
					context.contextId === executionContextId
				);
			},
		);
		page.on("Runtime.executionContextsCleared", (_, sessionId) => {
			this.#forget((context) => context.sessionId === sessionId);
		});
		page.on("Target.detachedFromTarget", ({ sessionId }) => {
			this.#forget((context) => context.sessionId === sessionId);
		});
		page.on("Target.attachedToTarget", async ({ sessionId, targetInfo }) => {
			if (targetInfo.type === "iframe") {
				await Promise.all(
					["Runtime.enable", "Log.enable"].map((method) =>
						page.send(method, {}, sessionId)
					),
				);
			}
			await page.send("Runtime.runIfWaitingForDebugger", {}, sessionId);
		});
	}

	// Attaches every frame the page starts in a process of its own, paused
	// until its listeners are in place so none of its errors are missed.
	attach() {
		return this.#page.send("Target.setAutoAttach", {
			autoAttach: true,
			waitForDebuggerOnStart: true,
			flatten: true,
		});
	}

	#forget(matches) {
		for (const [frameId, context] of this.#contexts) {
			if (matches(context)) this.#contexts.delete(frameId);
		}
	}

	// The id of the frame the `<iframe>` that `selector` names shows.
	async of(selector) {
		const { result } = await this.#page.send("Runtime.evaluate", {
			expression: `document.querySelector(${JSON.stringify(selector)})`,
		});
		assert(result.objectId, `no frame matches ${selector}`);
		const { node } = await this.#page.send("DOM.describeNode", {
			objectId: result.objectId,
		});
		await this.#page.send("Runtime.releaseObject", {
			objectId: result.objectId,
		});
		assert(node.frameId, `${selector} shows no frame`);
		return node.frameId;
	}

	// Evaluates `expression` in the frame `frameId` and returns its value.
	async evaluate(frameId, expression) {
		const context = this.#contexts.get(frameId);
		if (!context) throw new Error("the frame has no document yet");
		const { result, exceptionDetails } = await this.#page.send(
			"Runtime.evaluate",
			{ expression, contextId: context.contextId, returnByValue: true },
			context.sessionId,
		);
		if (exceptionDetails) {
			throw new Error(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		}
		return result.value;
	}
}

/**
 * A player on the page: its frame, and a reader's actions on the app inside
 * it. Points inside the app are mapped through the player's scaling to the
 * page, where the input is sent.
 */
class Player {
	constructor(page, frames, id, frameId) {
		this.page = page;
		this.frames = frames;
		this.id = id;
		this.frameId = frameId;
	}

	until(expression, description, timeout) {
		return waitFor(
			() => this.frames.evaluate(this.frameId, `!!(${expression})`),
			`${this.id}: ${description}`,
			timeout,
		);
	}

	/**
	 * Where on the page to point at the element `target` finds: once the
	 * element and its player have stopped moving, and the element is what
	 * the browser hits there.
	 */
	async point(target) {
		let last;
		return await waitFor(async () => {
			const inside = await this.frames.evaluate(
				this.frameId,
				`(() => {
					const element = ${target};
					if (!element) return null;
					const box = element.getBoundingClientRect();
					const x = box.x + box.width / 2;
					const y = box.y + box.height / 2;
					return {
						x,
						y,
						box: [box.x, box.y, box.width, box.height].join(),
						hit: element.contains(document.elementFromPoint(x, y)),
					};
				})()`,
			);
			if (!inside?.hit) return null;
			const frame = await this.page.evaluate(`(() => {
				const frame = document.querySelector("#${this.id} .lk-play-frame");
				const box = frame.getBoundingClientRect();
				const scale = box.width / frame.offsetWidth;
				const x = box.x + ${inside.x} * scale;
				const y = box.y + ${inside.y} * scale;
				return {
					x,
					y,
					box: [box.x, box.y, box.width, box.height].join(),
					// Off screen, or under the site's sticky header.
					covered: document.elementFromPoint(x, y) !== frame,
				};
			})()`);
			if (frame.covered) {
				await this.page.evaluate(`scrollBy(0, ${frame.y} - innerHeight / 2)`);
				return null;
			}
			const key = `${inside.box} ${frame.box}`;
			const settled = key === last;
			last = key;
			return settled && { x: frame.x, y: frame.y };
		}, `${this.id}: ${target} settling where it can be reached`);
	}

	// Presses the element `target` finds with the mouse.
	async press(target) {
		const { x, y } = await this.point(target);
		await this.page.send("Input.dispatchMouseEvent", {
			type: "mouseMoved",
			x,
			y,
		});
		for (const [type, buttons] of [["mousePressed", 1], ["mouseReleased", 0]]) {
			await this.page.send("Input.dispatchMouseEvent", {
				type,
				x,
				y,
				button: "left",
				buttons,
				clickCount: 1,
			});
		}
	}

	// Types `text` into whatever has focus.
	type(text) {
		return this.page.send("Input.insertText", { text });
	}

	// Scrolls the element `target` finds with the mouse wheel, and waits for
	// its content to move.
	async scroll(target) {
		const { x, y } = await this.point(target);
		await this.page.send("Input.dispatchMouseEvent", {
			type: "mouseWheel",
			x,
			y,
			deltaX: 0,
			deltaY: 400,
		});
		await this.until(
			`${target}?.scrollTop > 0`,
			"the scroll view moving its content",
		);
	}
}

/**
 * Presses a player's toolbar control, which a tall player's centring can
 * leave under the site's sticky header.
 */
async function pressToolbar(page, selector) {
	await page.evaluate(
		`document.querySelector(${
			JSON.stringify(selector)
		}).scrollIntoView({ block: "center", behavior: "instant" })`,
	);
	await page.click(selector);
}

/**
 * Opens `path` of the packaged site in a fresh headless Chrome and runs
 * `check` with the page, its frames, and the errors the page and its frames
 * have logged so far. Fails when `check` does, with those errors.
 */
async function browse(path, check) {
	// Fail rather than skip, so a CI run that lost the artifact cannot pass.
	assert(site, "set LEPTOSKIT_SITE to a packaged site, such as docs/dist");
	const server = serve(site);
	const profile = await Deno.makeTempDir({
		prefix: "leptoskit-playable-examples-",
	});
	const chrome = await launch(profile);
	const errors = [];
	let page;
	try {
		page = await Page.connect(chrome.endpoint);
		const frames = new Frames(page);
		page.on("Runtime.exceptionThrown", ({ exceptionDetails }) => {
			errors.push(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		});
		page.on("Runtime.consoleAPICalled", ({ type, args }) => {
			if (type === "error" || type === "assert") {
				errors.push(
					args.map((arg) => arg.value ?? arg.description).join(" "),
				);
			}
		});
		page.on("Log.entryAdded", ({ entry }) => {
			if (entry.level === "error") {
				errors.push(`${entry.text} ${entry.url ?? ""}`);
			}
		});
		await Promise.all(
			["Page.enable", "Runtime.enable", "Log.enable", "DOM.enable"].map(
				(method) => page.send(method),
			),
		);
		await frames.attach();

		const loaded = page.once("Page.loadEventFired");
		await page.send("Page.navigate", {
			url: `http://127.0.0.1:${server.addr.port}${path}`,
		});
		await loaded;
		await check(page, frames, errors);
	} catch (error) {
		throw new Error(`${error.message}\npage errors:\n${errors.join("\n")}`, {
			cause: error,
		});
	} finally {
		await page?.close();
		chrome.process.kill();
		await chrome.process.status;
		await server.shutdown();
		await removeProfile(profile);
	}
}

// Every player on the page, by id, in the order they appear.
function playerIds(page) {
	return page.evaluate(
		`[...document.querySelectorAll("[data-lk-play]")].map((figure) => figure.id)`,
	);
}

/**
 * Plays every player on the page in turn: scrolls it into view, waits for
 * its app to mount, and plays its interaction.
 */
async function playAll(page, frames, errors) {
	const ids = await playerIds(page);
	assert(ids.length > 0, "the page should play examples");
	const unknown = ids.filter((id) => !(id in PLAYS));
	assert(
		unknown.length === 0,
		`give these players an interaction in PLAYS: ${unknown.join(", ")}`,
	);
	// Players load as they near the viewport, so the first is still waiting.
	assert(
		await page.evaluate(
			`document.getElementById(${
				JSON.stringify(ids.at(-1))
			}).dataset.state === "idle"`,
		),
		"a player far below the viewport should not load yet",
	);

	for (const id of ids) {
		await page.evaluate(
			`document.getElementById(${
				JSON.stringify(id)
			}).scrollIntoView({ block: "center", behavior: "instant" })`,
		);
		await page.until(
			`() => {
				const state = document.getElementById(${
				JSON.stringify(id)
			}).dataset.state;
				if (state === "failed" || state === "missing") {
					throw new Error(${JSON.stringify(id)} + " could not start: " + state);
				}
				return state === "ready";
			}`,
			`${id} starting its app`,
			MOUNT_TIMEOUT,
		);
		const frameId = await frames.of(`#${id} .lk-play-frame`);
		const player = new Player(page, frames, id, frameId);
		await player.until(
			PLAYS[id].target,
			"the app mounting in its frame",
			MOUNT_TIMEOUT,
		);
		await PLAYS[id].play(player, page);
	}

	assert(
		errors.length === 0,
		`the page or its frames logged errors:\n${errors.join("\n")}`,
	);
}

Deno.test({
	name: "every example on the examples page plays",
	async fn() {
		await browse("/docs/examples.html", playAll);
	},
});

Deno.test({
	name: "every example on the home page plays",
	async fn() {
		await browse("/docs/", playAll);
	},
});

Deno.test({
	name: "a player switches to its AppKit image and back",
	async fn() {
		await browse("/docs/examples.html", async (page, _frames, errors) => {
			// The image is the AppKit render. The test server allows only the
			// site's own images, so pick a player whose image the site serves.
			const id = await page.evaluate(`(() => {
				const image = [...document.querySelectorAll(".lk-play-native")]
					.find((image) => new URL(image.dataset.src, location.href).origin === location.origin);
				return image?.closest("[data-lk-play]").id;
			})()`);
			assert(id, "a player should offer an AppKit image the site serves");
			const figure = `document.getElementById(${JSON.stringify(id)})`;
			await page.evaluate(
				`${figure}.scrollIntoView({ block: "center", behavior: "instant" })`,
			);
			await page.until(
				`() => ${figure}.dataset.state === "ready"`,
				`${id} starting its app`,
				MOUNT_TIMEOUT,
			);

			await pressToolbar(page, `#${id} [data-lk-play-view="native"]`);
			await page.until(
				`() => {
					const image = ${figure}.querySelector(".lk-play-native");
					return ${figure}.dataset.view === "native" && !image.hidden &&
						image.complete && image.naturalWidth > 0 &&
						getComputedStyle(${figure}.querySelector(".lk-play-frame")).visibility === "hidden" &&
						${figure}.querySelector("[data-lk-play-view='native']").getAttribute("aria-pressed") === "true" &&
						${figure}.querySelector("[data-lk-play-label]").textContent.includes("AppKit");
				}`,
				`${id} showing its AppKit image`,
			);

			await pressToolbar(page, `#${id} [data-lk-play-view="web"]`);
			await page.until(
				`() => ${figure}.dataset.view === "web" &&
					${figure}.querySelector(".lk-play-native").hidden &&
					getComputedStyle(${figure}.querySelector(".lk-play-frame")).visibility === "visible" &&
					${figure}.querySelector("[data-lk-play-label]").textContent.includes("browser")`,
				`${id} showing its web build again`,
			);
			assert(
				errors.length === 0,
				`the page or its frames logged errors:\n${errors.join("\n")}`,
			);
		});
	},
});
