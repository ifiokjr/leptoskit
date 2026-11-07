// The packaged website's Wasm client, in headless Chrome. On the landing
// page, the client must mount the live example over the static preview, its
// button must update the count, the page must log no errors, and the page
// must not download the management console, which loads as a chunk of its
// own. A client bundle without the app's start function still loads cleanly,
// so only a real browser notices that the example never became interactive.
//
// Package the site first, then point the test at the artifact (the test fails
// without one):
//
//   LEPTOSKIT_SITE=docs/dist deno test --allow-env --allow-read --allow-net \
//     --allow-run --allow-write crates/leptoskit_app/tests/live_example.mjs
//
// The complete hosted artifact (packaged without `--public-only`) also has
// the console at `/console/`; for it, the console tests check that the
// console loads its chunk and mounts, and that a chunk that fails to load
// leaves a message instead of an empty page. The public artifact has no
// console, so they are skipped there.
//
// `CHROME` names the browser executable; otherwise the usual install
// locations are tried. See `browser.mjs` for how the test drives Chrome.

import { assert, launch, Page, removeProfile, serve } from "./browser.mjs";

const site = Deno.env.get("LEPTOSKIT_SITE")?.replace(/\/$/, "");

function hasConsole() {
	try {
		return Deno.statSync(`${site}/console/index.html`).isFile;
	} catch {
		return false;
	}
}

// The client's Wasm modules the page requested, by path. The Rive runtime's
// own module, under `/rive/`, is not the client's.
const REQUESTED_WASM = `performance.getEntriesByType("resource")
	.map(({ name }) => new URL(name).pathname)
	.filter((path) => path.startsWith("/pkg/") && path.endsWith(".wasm"))`;

/**
 * Opens `path` of the packaged site in a fresh headless Chrome and runs
 * `check` with the page and the errors it has logged so far. `blocked` lists
 * URL patterns the browser must fail to fetch. Fails when `check` does, with
 * the page's errors and `shown` (an expression for what the page displays)
 * to explain it.
 */
async function browse({ path, blocked = [], shown, check }) {
	// Fail rather than skip, so a CI run that lost the artifact cannot pass.
	assert(site, "set LEPTOSKIT_SITE to a packaged site, such as docs/dist");
	const server = serve(site);
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
			["Page.enable", "Runtime.enable", "Log.enable", "Network.enable"].map(
				(m) => page.send(m),
			),
		);
		if (blocked.length > 0) {
			await page.send("Network.setBlockedURLs", { urls: blocked });
		}

		const loaded = page.once("Page.loadEventFired");
		await page.send("Page.navigate", {
			url: `http://127.0.0.1:${server.addr.port}${path}`,
		});
		await loaded;
		await check(page, errors);
	} catch (error) {
		// The page's own errors usually explain a stalled client.
		const display = page ? await page.evaluate(shown) : "";
		throw new Error(
			`${error.message}\nthe page shows: ${display}\npage errors:\n${
				errors.join("\n")
			}`,
			{ cause: error },
		);
	} finally {
		await page?.close();
		chrome.process.kill();
		await chrome.process.status;
		await server.shutdown();
		await removeProfile(profile);
	}
}

Deno.test({
	name: "the packaged live example mounts and counts presses",
	async fn() {
		await browse({
			path: "/",
			shown: `document.querySelector("#live-demo")?.textContent`,
			async check(page, errors) {
				// The server-rendered preview is inert until the client mounts.
				assert(
					await page.evaluate(
						`document.querySelector("#live-demo [aria-disabled='true']") !== null`,
					),
					"the static preview should wait for the Wasm client",
				);

				// The client loads lazily, as the example nears the viewport.
				// The page scrolls smoothly; jump instead, so the example
				// settles at once.
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

				// The console's chunk is for the console alone.
				const wasm = await page.evaluate(REQUESTED_WASM);
				assert(
					wasm.length === 1 && wasm[0] === "/pkg/leptoskit_app_bg.wasm",
					`the landing page should load only the client's main module, not ${
						wasm.join(", ")
					}`,
				);
				assert(
					errors.length === 0,
					`the page logged errors:\n${errors.join("\n")}`,
				);
			},
		});
	},
});

Deno.test({
	name: "the packaged console loads its chunk and mounts",
	ignore: !site || !hasConsole(),
	async fn() {
		await browse({
			path: "/console/",
			shown: "document.body.textContent",
			async check(page, errors) {
				await page.until(
					`() => document.querySelector("[data-testid='account-key']")`,
					"the console mounting its sign-in form",
				);
				const wasm = await page.evaluate(REQUESTED_WASM);
				assert(
					wasm.includes("/pkg/leptoskit_app_bg.wasm") &&
						wasm.some((path) => path.startsWith("/pkg/split_")),
					`the console should load the main module and its chunk, not ${
						wasm.join(", ")
					}`,
				);
				assert(
					errors.length === 0,
					`the page logged errors:\n${errors.join("\n")}`,
				);
			},
		});
	},
});

Deno.test({
	name: "a console chunk that fails to load leaves a message",
	ignore: !site || !hasConsole(),
	async fn() {
		await browse({
			path: "/console/",
			blocked: ["*/pkg/split_*"],
			shown: "document.body.textContent",
			async check(page, errors) {
				await page.until(
					`() => document.body.textContent.includes("LeptosKit could not load.")`,
					"the console reporting that it could not load",
				);
				// `bootstrap.js` shows the same message when the main module
				// fails, so make sure the client started and its chunk loader
				// reported the failed fetch: the message is then the client's.
				assert(
					errors.some((error) => error.includes("/pkg/__wasm_split.")),
					"the client should have tried to load the console's chunk",
				);
			},
		});
	},
});
