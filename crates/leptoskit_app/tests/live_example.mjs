// The packaged public website's live example, in headless Chrome: the Wasm
// client must mount over the static preview, its button must update the
// count, and the page must log no errors. A client bundle without the app's
// start function still loads cleanly, so only a real browser notices that the
// example never became interactive.
//
// Package the site first, then point the test at the artifact (the test fails
// without one):
//
//   LEPTOSKIT_SITE=docs/dist deno test --allow-env --allow-read --allow-net \
//     --allow-run --allow-write crates/leptoskit_app/tests/live_example.mjs
//
// `CHROME` names the browser executable; otherwise the usual install
// locations are tried. The test talks to Chrome over the DevTools protocol
// directly, so it needs no browser automation package.

const site = Deno.env.get("LEPTOSKIT_SITE");

// The broker's policy, so the client is also checked under the hosted
// artifact's restrictions.
const CONTENT_SECURITY_POLICY =
	"default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; " +
	"style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

const CONTENT_TYPES = {
	css: "text/css; charset=utf-8",
	html: "text/html; charset=utf-8",
	ico: "image/x-icon",
	js: "text/javascript; charset=utf-8",
	json: "application/json",
	mjs: "text/javascript; charset=utf-8",
	png: "image/png",
	riv: "application/octet-stream",
	svg: "image/svg+xml",
	txt: "text/plain; charset=utf-8",
	wasm: "application/wasm",
	woff2: "font/woff2",
	xml: "application/xml",
};

function assert(value, message) {
	if (!value) throw new Error(message);
}

// Serves `root` as static files, the way the public host does: a directory
// resolves to its `index.html`.
function serve(root) {
	return Deno.serve(
		{ hostname: "127.0.0.1", port: 0, onListen() {} },
		async (request) => {
			const path = decodeURIComponent(new URL(request.url).pathname);
			if (path.split("/").includes("..")) {
				return new Response("Bad request", { status: 400 });
			}
			let file = `${root}${path}`;
			if (path.endsWith("/")) file += "index.html";
			try {
				const body = await Deno.readFile(file);
				const extension = file.slice(file.lastIndexOf(".") + 1);
				return new Response(body, {
					headers: {
						"content-type": CONTENT_TYPES[extension] ??
							"application/octet-stream",
						"content-security-policy": CONTENT_SECURITY_POLICY,
					},
				});
			} catch (error) {
				if (error instanceof Deno.errors.NotFound) {
					return new Response("Not found", { status: 404 });
				}
				throw error;
			}
		},
	);
}

function chromeExecutable() {
	const configured = Deno.env.get("CHROME");
	if (configured) return configured;
	const candidates = Deno.build.os === "darwin"
		? [
			"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
			"/Applications/Chromium.app/Contents/MacOS/Chromium",
		]
		: [
			"/usr/bin/google-chrome",
			"/usr/bin/google-chrome-stable",
			"/usr/bin/chromium",
			"/usr/bin/chromium-browser",
		];
	const found = candidates.find((candidate) => {
		try {
			return Deno.statSync(candidate).isFile;
		} catch {
			return false;
		}
	});
	assert(found, "Chrome was not found; set CHROME to its executable");
	return found;
}

// Starts headless Chrome and returns it with its DevTools endpoint.
async function launch(profile) {
	const args = [
		"--headless=new",
		"--remote-debugging-port=0",
		`--user-data-dir=${profile}`,
		"--no-first-run",
		"--no-default-browser-check",
		"--window-size=1280,900",
		"about:blank",
	];
	// CI runners do not give Chrome the user namespaces its sandbox needs.
	if (Deno.build.os === "linux") args.unshift("--no-sandbox");
	const process = new Deno.Command(chromeExecutable(), {
		args,
		stdin: "null",
		stdout: "null",
		stderr: "piped",
	}).spawn();
	const reader = process.stderr.pipeThrough(new TextDecoderStream())
		.getReader();
	let output = "";
	try {
		for (;;) {
			const { value, done } = await reader.read();
			if (done) {
				throw new Error(`Chrome exited before DevTools started:\n${output}`);
			}
			output += value;
			const endpoint = output.match(/DevTools listening on (ws:\/\/\S+)/)?.[1];
			if (endpoint) return { process, endpoint };
		}
	} finally {
		await reader.cancel();
	}
}

// One DevTools protocol connection to a page.
class Page {
	#socket;
	#next = 0;
	#pending = new Map();
	#listeners = new Map();

	static async connect(browserEndpoint) {
		const { host } = new URL(browserEndpoint);
		const targets = await (await fetch(`http://${host}/json/list`)).json();
		const page = targets.find((target) => target.type === "page");
		assert(page, "Chrome opened no page");
		const socket = new WebSocket(page.webSocketDebuggerUrl);
		await new Promise((resolve, reject) => {
			socket.onopen = resolve;
			socket.onerror = () => reject(new Error("could not connect to Chrome"));
		});
		return new Page(socket);
	}

	constructor(socket) {
		this.#socket = socket;
		socket.onmessage = ({ data }) => {
			const message = JSON.parse(data);
			if (message.id === undefined) {
				for (const listener of this.#listeners.get(message.method) ?? []) {
					listener(message.params);
				}
				return;
			}
			const pending = this.#pending.get(message.id);
			this.#pending.delete(message.id);
			if (message.error) pending.reject(new Error(message.error.message));
			else pending.resolve(message.result);
		};
	}

	send(method, params = {}) {
		const id = ++this.#next;
		this.#socket.send(JSON.stringify({ id, method, params }));
		return new Promise((resolve, reject) =>
			this.#pending.set(id, { resolve, reject })
		);
	}

	on(method, listener) {
		const listeners = this.#listeners.get(method) ?? [];
		listeners.push(listener);
		this.#listeners.set(method, listeners);
	}

	once(method) {
		return new Promise((resolve) => this.on(method, resolve));
	}

	// Evaluates `expression` in the page and returns its value.
	async evaluate(expression) {
		const { result, exceptionDetails } = await this.send("Runtime.evaluate", {
			expression,
			awaitPromise: true,
			returnByValue: true,
		});
		if (exceptionDetails) {
			throw new Error(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		}
		return result.value;
	}

	// Waits, inside the page, until `predicate` returns a truthy value.
	until(predicate, description, timeout = 20_000) {
		return this.evaluate(`new Promise((resolve, reject) => {
			const deadline = performance.now() + ${timeout};
			const check = () => {
				const value = (${predicate})();
				if (value) resolve(value);
				else if (performance.now() > deadline) reject(new Error(${
			JSON.stringify(`timed out: ${description}`)
		}));
				else setTimeout(check, 50);
			};
			check();
		})`);
	}

	// Presses the element `selector` names with the mouse, as a reader would:
	// once its entrance motion has settled and nothing covers its centre.
	async click(selector) {
		const { x, y } = await this.until(
			`() => {
				const element = document.querySelector(${JSON.stringify(selector)});
				const box = element?.getBoundingClientRect();
				const key = box && [box.x, box.y, box.width, box.height].join();
				const settled = key === globalThis.__lastBox;
				globalThis.__lastBox = key;
				if (!settled) return null;
				const x = box.x + box.width / 2;
				const y = box.y + box.height / 2;
				return element.contains(document.elementFromPoint(x, y)) && { x, y };
			}`,
			`${selector} settling where it can be pressed`,
		);
		await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
		await this.send("Input.dispatchMouseEvent", {
			type: "mousePressed",
			x,
			y,
			button: "left",
			buttons: 1,
			clickCount: 1,
		});
		await this.send("Input.dispatchMouseEvent", {
			type: "mouseReleased",
			x,
			y,
			button: "left",
			buttons: 0,
			clickCount: 1,
		});
	}

	async close() {
		const closed = new Promise((resolve) => this.#socket.onclose = resolve);
		this.#socket.close();
		await closed;
	}
}

Deno.test({
	name: "the packaged live example mounts and counts presses",
	async fn() {
		// Fail rather than skip, so a CI run that lost the artifact cannot pass.
		assert(site, "set LEPTOSKIT_SITE to a packaged site, such as docs/dist");
		const server = serve(site.replace(/\/$/, ""));
		const profile = await Deno.makeTempDir({
			prefix: "leptoskit-live-example-",
		});
		const chrome = await launch(profile);
		const errors = [];
		let page;
		try {
			page = await Page.connect(chrome.endpoint);
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
				["Page.enable", "Runtime.enable", "Log.enable"].map((m) =>
					page.send(m)
				),
			);

			const loaded = page.once("Page.loadEventFired");
			await page.send("Page.navigate", {
				url: `http://127.0.0.1:${server.addr.port}/`,
			});
			await loaded;

			// The server-rendered preview is inert until the client mounts.
			assert(
				await page.evaluate(
					`document.querySelector("#live-demo [aria-disabled='true']") !== null`,
				),
				"the static preview should wait for the Wasm client",
			);

			// The client loads lazily, as the example nears the viewport. The
			// page scrolls smoothly; jump instead, so the example settles at once.
			await page.evaluate(
				`document.getElementById("live-demo").scrollIntoView({ block: "center", behavior: "instant" })`,
			);
			await page.until(
				`() => document.querySelector("#live-demo #demo-increment") &&
					!document.querySelector("#live-demo [aria-disabled='true']")`,
				"the Wasm client mounting the live example",
			);
			const count = () =>
				page.evaluate(
					`document.querySelector("#live-demo #demo-count")?.textContent`,
				);
			assert(
				(await count()) === "0 ideas shipped",
				`unexpected count: ${await count()}`,
			);

			for (const shipped of [1, 2]) {
				await page.click("#live-demo #demo-increment");
				await page.until(
					`() => document.querySelector("#live-demo #demo-count")?.textContent === "${shipped} ideas shipped"`,
					`the count reaching ${shipped}`,
				);
			}
		} catch (error) {
			// The page's own errors usually explain a stalled example.
			const shown = page
				? await page.evaluate(
					`document.querySelector("#live-demo")?.textContent`,
				)
				: "";
			throw new Error(
				`${error.message}\nlive example: ${shown}\npage errors:\n${
					errors.join("\n")
				}`,
				{
					cause: error,
				},
			);
		} finally {
			await page?.close();
			chrome.process.kill();
			await chrome.process.status;
			await server.shutdown();
			await Deno.remove(profile, { recursive: true });
		}
		assert(
			errors.length === 0,
			`the page logged errors:\n${errors.join("\n")}`,
		);
	},
});
