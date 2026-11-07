// Times the packaged public website's live example on a throttled phone:
// how long after the example scrolls into view its button first answers a
// press, and what the page downloads for it. Each run starts Chrome with an
// empty cache, emulates Lighthouse's mobile device and its slow 4G and 4x CPU
// slowdown, waits for the page to settle, and only then scrolls.
//
//   LEPTOSKIT_SITE=docs/dist deno run --allow-env --allow-read --allow-net \
//     --allow-run --allow-write crates/leptoskit_app/tests/live_example_timing.mjs
//
// `RUNS` sets the number of runs (default 7), `ENCODING` the response
// compression (`gzip`, as GitHub Pages sends it, by default; or `br`), and
// `OUT` a file to write every run's measurements to as JSON.
//
// Each run reports three times, in milliseconds from the scroll:
//
// - `mounted`: the Wasm client replaced the static preview;
// - `interactive`: a press made the moment it mounted updated the count;
// - `pressed`: a second, real mouse press updated the count, with the
//   press's Event Timing duration (`press_ms`), the INP-style latency.
//
// It also reports the long tasks after the scroll (`blocking_ms`, the
// Total Blocking Time over that window), whether the Rive runtime was ready
// when the example mounted (`rive_at_mount`; otherwise the mark falls back to
// its static image), and each request the example made.

import { assert, launch, Page, removeProfile, serve } from "./browser.mjs";

const site = Deno.env.get("LEPTOSKIT_SITE");
const runs = Number(Deno.env.get("RUNS") ?? 7);
const encoding = Deno.env.get("ENCODING") ?? "gzip";
const out = Deno.env.get("OUT");

// Lighthouse 12's mobile preset for DevTools throttling: a Moto G Power on
// slow 4G (150 ms round trips, 1.6 Mbps down and 750 Kbps up, with the
// DevTools adjustment factors applied) and a 4x slower CPU.
const DEVICE = {
	width: 412,
	height: 823,
	deviceScaleFactor: 1.75,
	mobile: true,
};
const NETWORK = {
	offline: false,
	latency: 150 * 3.75,
	downloadThroughput: (1.6 * 1024 * 0.9 * 1024) / 8,
	uploadThroughput: (750 * 0.9 * 1024) / 8,
};
const CPU_SLOWDOWN = 4;

// Installed before the page's own scripts: long tasks and event timings are
// buffered from the start, so the measurement window can be read later.
const OBSERVE = `
	globalThis.__longTasks = [];
	new PerformanceObserver((list) => globalThis.__longTasks.push(
		...list.getEntries().map(({ startTime, duration }) => ({ startTime, duration })),
	)).observe({ type: "longtask", buffered: true });
	globalThis.__events = [];
	new PerformanceObserver((list) => globalThis.__events.push(
		...list.getEntries().map((entry) => ({
			name: entry.name,
			startTime: entry.startTime,
			duration: entry.duration,
			processingStart: entry.processingStart,
			processingEnd: entry.processingEnd,
			interactionId: entry.interactionId,
		})),
	)).observe({ type: "event", durationThreshold: 16, buffered: true });
`;

// Scrolls to the example and resolves once a press made the moment it
// mounts has updated the count.
const SCROLL_AND_PRESS = `new Promise((resolve, reject) => {
	const demo = document.getElementById("live-demo");
	const live = () => demo.querySelector("#demo-increment") &&
		!demo.querySelector("[aria-disabled='true']");
	const count = () => demo.querySelector("#demo-count")?.textContent;
	let mounted;
	let rive;
	const observer = new MutationObserver(() => {
		if (mounted === undefined && live()) {
			mounted = performance.now();
			rive = demo.querySelector("#demo-mark canvas, canvas#demo-mark") !== null;
			demo.querySelector("#demo-increment").click();
		}
		if (mounted !== undefined && count() === "1 ideas shipped") {
			observer.disconnect();
			resolve({ start, mounted, rive, interactive: performance.now() });
		}
	});
	observer.observe(demo, {
		subtree: true,
		childList: true,
		characterData: true,
		attributes: true,
	});
	setTimeout(() => reject(new Error("the example did not answer within 60 s")), 60_000);
	const start = performance.now();
	demo.scrollIntoView({ block: "center", behavior: "instant" });
})`;

async function measure(origin) {
	const profile = await Deno.makeTempDir({ prefix: "leptoskit-timing-" });
	const chrome = await launch(profile);
	let page;
	try {
		page = await Page.connect(chrome.endpoint);
		await Promise.all(
			["Page.enable", "Runtime.enable", "Network.enable"].map((method) =>
				page.send(method)
			),
		);
		await page.send("Network.setCacheDisabled", { cacheDisabled: true });
		await page.send("Emulation.setDeviceMetricsOverride", DEVICE);
		await page.send("Network.emulateNetworkConditions", NETWORK);
		await page.send("Emulation.setCPUThrottlingRate", {
			rate: CPU_SLOWDOWN,
		});
		await page.send("Page.addScriptToEvaluateOnNewDocument", {
			source: OBSERVE,
		});

		// Requests in flight, so the scroll waits for a quiet network.
		const inFlight = new Set();
		let lastActivity = performance.now();
		page.on("Network.requestWillBeSent", ({ requestId }) => {
			inFlight.add(requestId);
			lastActivity = performance.now();
		});
		for (const event of ["Network.loadingFinished", "Network.loadingFailed"]) {
			page.on(event, ({ requestId }) => {
				inFlight.delete(requestId);
				lastActivity = performance.now();
			});
		}

		const loaded = page.once("Page.loadEventFired");
		await page.send("Page.navigate", { url: `${origin}/` });
		await loaded;
		while (inFlight.size > 0 || performance.now() - lastActivity < 2_000) {
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
		assert(
			await page.evaluate(
				`performance.getEntriesByType("resource").every(({ name }) => !name.includes("/bootstrap.js"))`,
			),
			"the client loaded before the example scrolled into view",
		);

		const { start, mounted, rive, interactive } = await page.evaluate(
			SCROLL_AND_PRESS,
		);
		await page.click("#live-demo #demo-increment", 60_000);
		await page.until(
			`() => document.querySelector("#live-demo #demo-count")?.textContent === "2 ideas shipped"`,
			"the second press updating the count",
			60_000,
		);
		const pressed = await page.evaluate("performance.now()");

		// Let Rive's runtime and anything else the example started finish,
		// so the long tasks and requests below are complete.
		while (inFlight.size > 0 || performance.now() - lastActivity < 2_000) {
			await new Promise((resolve) => setTimeout(resolve, 100));
		}
		const window = await page.evaluate(`(() => {
			const start = ${start};
			const longTasks = globalThis.__longTasks.filter((task) => task.startTime >= start);
			const press = globalThis.__events
				.filter((entry) => entry.interactionId && entry.startTime >= ${interactive})
				.reduce((longest, entry) => Math.max(longest, entry.duration), 0);
			const requests = performance.getEntriesByType("resource")
				.filter((entry) => entry.startTime >= start)
				.map((entry) => ({
					path: new URL(entry.name).pathname,
					start: Math.round(entry.startTime - start),
					end: Math.round(entry.responseEnd - start),
					transferred: entry.transferSize,
					encoded: entry.encodedBodySize,
					decoded: entry.decodedBodySize,
				}));
			return {
				longTasks: longTasks.length,
				blocking: longTasks.reduce((sum, task) => sum + Math.max(0, task.duration - 50), 0),
				press,
				requests,
			};
		})()`);
		return {
			mounted_ms: Math.round(mounted - start),
			rive_at_mount: rive,
			interactive_ms: Math.round(interactive - start),
			pressed_ms: Math.round(pressed - start),
			press_ms: Math.round(window.press),
			long_tasks: window.longTasks,
			blocking_ms: Math.round(window.blocking),
			requests: window.requests,
		};
	} finally {
		await page?.close();
		chrome.process.kill();
		await chrome.process.status;
		await removeProfile(profile);
	}
}

function median(values) {
	const sorted = [...values].sort((a, b) => a - b);
	const middle = Math.floor(sorted.length / 2);
	return sorted.length % 2
		? sorted[middle]
		: Math.round((sorted[middle - 1] + sorted[middle]) / 2);
}

assert(site, "set LEPTOSKIT_SITE to a packaged site, such as docs/dist");
const server = serve(site.replace(/\/$/, ""), { encoding });
const results = [];
try {
	const origin = `http://127.0.0.1:${server.addr.port}`;
	for (let run = 1; run <= runs; run++) {
		const result = await measure(origin);
		results.push(result);
		console.log(
			`run ${run}: mounted ${result.mounted_ms} ms, interactive ${result.interactive_ms} ms, ` +
				`second press ${result.pressed_ms} ms (event ${result.press_ms} ms), ` +
				`blocking ${result.blocking_ms} ms over ${result.long_tasks} long tasks, ` +
				`Rive ${result.rive_at_mount ? "ready" : "missing"} at mount`,
		);
	}
} finally {
	await server.shutdown();
}

console.log(
	`\nmedian of ${runs} runs (${encoding}, slow 4G, ${CPU_SLOWDOWN}x CPU):`,
);
for (
	const key of [
		"mounted_ms",
		"interactive_ms",
		"pressed_ms",
		"press_ms",
		"blocking_ms",
	]
) {
	console.log(
		`  ${key.padEnd(15)} ${median(results.map((result) => result[key]))}`,
	);
}
console.log("\nrequests after the scroll (first run):");
for (const request of results[0].requests) {
	console.log(
		`  ${request.path.padEnd(48)} ${
			String(request.transferred).padStart(9)
		} B over the wire, ` +
			`${
				String(request.decoded).padStart(9)
			} B decoded, ${request.start}–${request.end} ms`,
	);
}
if (out) {
	await Deno.writeTextFile(
		out,
		JSON.stringify(
			{ encoding, network: NETWORK, cpu: CPU_SLOWDOWN, results },
			null,
			"\t",
		),
	);
}
