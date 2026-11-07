// Drives headless Chrome over the DevTools protocol directly, so the
// packaged site's browser checks need no automation package: a static file
// server shaped like the public host, Chrome's launch and cleanup, and one
// page connection.
//
// `CHROME` names the browser executable; otherwise the usual install
// locations are tried.

import { brotliCompressSync, constants, gzipSync } from "node:zlib";

// The broker's policy, so the client is also checked under the hosted
// artifact's restrictions.
const CONTENT_SECURITY_POLICY =
	"default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; " +
	"style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; object-src 'none'";

// The broker's policy for the examples the book plays, under `/docs/play/`:
// the book's pages may frame them, they name where they are served with
// `<base href>`, and the gallery draws its images from `data:` URLs.
const PLAYED_EXAMPLE_POLICY =
	"default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; " +
	"style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; " +
	"frame-ancestors 'self'; base-uri 'self'; object-src 'none'";

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

// Already-compressed formats the public host sends as they are.
const PRECOMPRESSED = new Set(["png", "woff2", "ico", "riv"]);

export function assert(value, message) {
	if (!value) throw new Error(message);
}

function compress(body, encoding) {
	if (encoding === "br") {
		return brotliCompressSync(body, {
			params: { [constants.BROTLI_PARAM_QUALITY]: 11 },
		});
	}
	return gzipSync(body, { level: 9 });
}

/**
 * Serves `root` as static files, the way the public host does: a directory
 * resolves to its `index.html`, and any origin may read a file. With `encoding` (`"gzip"` or `"br"`), text
 * and Wasm responses are compressed with it whenever the browser accepts it,
 * as GitHub Pages (gzip) and Cloudflare (brotli) send them.
 */
export function serve(root, { encoding } = {}) {
	const compressed = new Map();
	return Deno.serve(
		{ hostname: "127.0.0.1", port: 0, onListen() {} },
		async (request) => {
			const path = decodeURIComponent(new URL(request.url).pathname);
			if (path.split("/").includes("..")) {
				return new Response("Bad request", { status: 400 });
			}
			let file = `${root}${path}`;
			if (path.endsWith("/")) file += "index.html";
			let body;
			try {
				body = await Deno.readFile(file);
			} catch (error) {
				if (error instanceof Deno.errors.NotFound) {
					return new Response("Not found", { status: 404 });
				}
				throw error;
			}
			const extension = file.slice(file.lastIndexOf(".") + 1);
			const played = path.startsWith("/docs/play/");
			const headers = {
				"content-type": CONTENT_TYPES[extension] ?? "application/octet-stream",
				"content-security-policy": played
					? PLAYED_EXAMPLE_POLICY
					: CONTENT_SECURITY_POLICY,
				// The public host lets any origin read its files, which a
				// sandboxed frame's opaque origin needs to load an example.
				"access-control-allow-origin": "*",
			};
			const accepted = (request.headers.get("accept-encoding") ?? "")
				.split(",")
				.map((name) => name.trim().split(";")[0]);
			if (
				encoding && accepted.includes(encoding) &&
				!PRECOMPRESSED.has(extension)
			) {
				const key = `${encoding}:${file}`;
				if (!compressed.has(key)) compressed.set(key, compress(body, encoding));
				body = compressed.get(key);
				headers["content-encoding"] = encoding;
				headers["vary"] = "accept-encoding";
			}
			return new Response(body, { headers });
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
export async function launch(profile) {
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
export class Page {
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
					listener(message.params, message.sessionId);
				}
				return;
			}
			const pending = this.#pending.get(message.id);
			this.#pending.delete(message.id);
			if (message.error) pending.reject(new Error(message.error.message));
			else pending.resolve(message.result);
		};
	}

	// Sends `method` to the page, or to the target attached as `sessionId`
	// (with `Target.setAutoAttach`'s `flatten`), such as a frame that runs in
	// a process of its own.
	send(method, params = {}, sessionId) {
		const id = ++this.#next;
		this.#socket.send(JSON.stringify({ id, method, params, sessionId }));
		return new Promise((resolve, reject) =>
			this.#pending.set(id, { resolve, reject })
		);
	}

	// Calls `listener` with each event's parameters and the session it came
	// from, which is undefined for the page's own.
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
	async click(selector, timeout) {
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
			timeout,
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

/**
 * Removes Chrome's temporary profile. Chrome's helper processes can outlive
 * the browser process for a moment and keep writing into the profile, so on
 * Linux the first removal can fail with "Directory not empty". Retry briefly
 * rather than let cleanup replace the caller's own result.
 */
export async function removeProfile(profile) {
	for (let attempt = 1;; attempt++) {
		try {
			await Deno.remove(profile, { recursive: true });
			return;
		} catch (error) {
			if (error instanceof Deno.errors.NotFound) return;
			if (attempt === 20) throw error;
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
	}
}
