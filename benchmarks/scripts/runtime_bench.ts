// The browser runtime benchmark: serves a built fixture, drives it in
// headless Chromium over CDP, and writes one JSON run.
//
// Every scenario is verified against the expected DOM state before its
// duration is recorded (the `isDone` predicate inside `__bench.run`), so a
// broken app times out instead of posting a fast number. Startup metrics
// come from the page's own navigation timing; payload metrics from the
// resource entries (gzip-served, like a real deployment).
//
// Usage (one subject):
//   deno run -A benchmarks/scripts/runtime_bench.ts \
//     --app leptoskit --dist <dir> --out results.json [--runs 7]
//
// Usage (several builds of the same app, interleaved per run so machine
// drift lands on all of them evenly — this is how base and head compare):
//   deno run -A benchmarks/scripts/runtime_bench.ts \
//     --app leptoskit --dist base=<dir> --dist head=<dir> --out-dir <dir>
//
// The app directory may carry a `bench.app.json` with `{ "domScenarios": false }`
// for renderers the DOM readers cannot verify (canvas-based, e.g. Flutter):
// those apps report startup, payload, and memory metrics only.

import { launchBrowser } from "./cdp.ts";
import type { Page } from "./cdp.ts";
import {
	aggregate,
	type BenchmarkRun,
	type CliArguments,
	emptyMetric,
	fail,
	formatMetric,
	log,
	optionalNumber,
	optionalString,
	parseArguments,
	requireString,
	RUNTIME_APPS_ROOT,
	syncBenchGlue,
} from "./common.ts";
import { serveStatic, type StaticServer } from "./serve.ts";

interface BeforeState {
	rows: number;
	label0: string | null;
	label1: string | null;
	label501: string | null;
	label998: string | null;
}

interface Scenario {
	id: string;
	button: string;
	/** The page-side completion predicate, with before-state interpolated. */
	predicate: (before: BeforeState) => string;
}

// Large enough that handler work dominates the measurement's fixed costs (the
// layout and predicate check that end it), so frameworks separate.
const CLICK_TOTAL = 50_000;

// The scenario order deliberately chains state: create → mutate the created
// rows → append → clear → create big → measure heap. Row ids are
// deterministic from a fresh page (1..1000, then 1001..2000, then
// 2001..12000), which the predicates bake in.
const SCENARIOS: Scenario[] = [
	{
		id: "table.create-1000",
		button: "create-1000",
		predicate: () =>
			`window.__bench.rows() === 1000 && window.__bench.label(999) === window.__bench.labelFor(window.__bench.rowId(999))`,
	},
	{
		id: "table.update-every-10th",
		button: "update-every-10th",
		predicate: (before) =>
			`window.__bench.rows() === 1000 && window.__bench.label(0) === ${
				JSON.stringify(before.label0)
			} + " !!!" && window.__bench.label(1) === ${
				JSON.stringify(before.label1)
			} && window.__bench.label(990).endsWith(" !!!")`,
	},
	{
		id: "table.swap-rows",
		button: "swap-rows",
		predicate: (before) =>
			`window.__bench.rows() === 1000 && window.__bench.label(1) === ${
				JSON.stringify(before.label998)
			} && window.__bench.label(998) === ${JSON.stringify(before.label1)}`,
	},
	{
		id: "table.remove-row",
		button: "remove-row",
		predicate: (before) =>
			`window.__bench.rows() === 999 && window.__bench.label(500) === ${
				JSON.stringify(before.label501)
			}`,
	},
	{
		id: "table.select-row",
		button: "select-row",
		predicate: () =>
			`window.__bench.rowClass(42).includes("selected") && !window.__bench.rowClass(41).includes("selected")`,
	},
	{
		id: "table.append-1000",
		button: "append-1000",
		predicate: () =>
			`window.__bench.rows() === 1999 && window.__bench.rowId(1998) === 2000 && window.__bench.label(1998) === window.__bench.labelFor(2000)`,
	},
	{
		id: "table.clear",
		button: "clear",
		predicate: () => `window.__bench.rows() === 0`,
	},
	{
		id: "table.create-10000",
		button: "create-10000",
		predicate: () =>
			`window.__bench.rows() === 10000 && window.__bench.rowId(9999) === 12000 && window.__bench.label(9999) === window.__bench.labelFor(12000)`,
	},
];

async function readBefore(page: Page): Promise<BeforeState> {
	return await page.evaluate<BeforeState>(`(() => ({
		rows: window.__bench.rows(),
		label0: window.__bench.label(0),
		label1: window.__bench.label(1),
		label501: window.__bench.label(501),
		label998: window.__bench.label(998),
	}))()`);
}

async function waitForReady(page: Page): Promise<void> {
	const deadline = Date.now() + 30_000;
	while (Date.now() < deadline) {
		const ready = await page.evaluate<boolean>(
			`Boolean(window.__bench && window.__bench.ready)`,
		);
		if (ready) return;
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
	throw new Error("the app never signalled mount within 30s");
}

interface Subject {
	name: string;
	dist: string;
	server: StaticServer;
	metrics: Record<string, ReturnType<typeof emptyMetric>>;
}

function newMetrics(): Record<string, ReturnType<typeof emptyMetric>> {
	const metrics: Record<string, ReturnType<typeof emptyMetric>> = {
		"startup.mounted": emptyMetric("ms", "startup", "time to first render"),
		"startup.transfer": emptyMetric(
			"bytes",
			"bytes",
			"compressed transfer for page load",
		),
		"startup.heap": emptyMetric("bytes", "memory", "JS heap after load"),
		"clicks.duration": emptyMetric(
			"ms",
			"duration",
			`duration of ${CLICK_TOTAL} clicks`,
		),
		"clicks.throughput": emptyMetric(
			"per-sec",
			"throughput",
			"increment clicks per second",
		),
		"memory.heap-after-create-10000": emptyMetric(
			"bytes",
			"memory",
			"JS heap after creating 10,000 rows",
		),
	};
	for (const scenario of SCENARIOS) {
		metrics[scenario.id] = emptyMetric("ms", "duration", scenario.id);
	}
	return metrics;
}

/** One fresh-page pass over every metric for a subject. */
async function measureIteration(
	page: Page,
	subject: Subject,
	domScenarios: boolean,
): Promise<string> {
	const { metrics } = subject;
	// Every load starts from an empty page with the previous one collected.
	// `startup.mounted` is timed from navigation start, and leaving the last
	// page (10,000 rows after `table.create-10000`) otherwise landed inside
	// it: identical builds read ~60 ms or ~190 ms per sample depending on when
	// that teardown and its garbage collection ran.
	await page.navigate("about:blank");
	await page.evaluate(`typeof globalThis.gc === "function" && globalThis.gc()`);
	await page.navigate(subject.server.url);
	await waitForReady(page);
	const userAgent = await page.evaluate<string>(`navigator.userAgent`);

	metrics["startup.mounted"].values.push(
		await page.evaluate<number>(`window.__bench.mountedAt ?? -1`),
	);
	metrics["startup.transfer"].values.push(
		await page.evaluate<number>(`window.__bench.transferBytes()`),
	);
	metrics["startup.heap"].values.push(
		await page.evaluate<number>(`window.__bench.heapBytes() ?? -1`),
	);

	if (domScenarios) {
		for (const scenario of SCENARIOS) {
			const before = await readBefore(page);
			const duration = await page.evaluate<number>(
				`window.__bench.run(${JSON.stringify(scenario.button)}, () => (${
					scenario.predicate(before)
				}))`,
			);
			metrics[scenario.id].values.push(duration);

			// Click throughput runs on the empty table, right after `clear`:
			// the measurement ends on two animation frames, and with 10,000
			// rows in the document each frame can cost a full table relayout
			// — which is layout cost, not the framework's event path, and on
			// slower machines it made this metric bimodal.
			if (scenario.id === "table.clear") {
				const clicksDuration = await page.evaluate<number>(
					`window.__bench.clicks(${CLICK_TOTAL})`,
				);
				metrics["clicks.duration"].values.push(clicksDuration);
				metrics["clicks.throughput"].values.push(
					(CLICK_TOTAL / clicksDuration) * 1000,
				);
			}

			if (scenario.id === "table.create-10000") {
				metrics["memory.heap-after-create-10000"].values.push(
					await page.evaluate<number>(`window.__bench.heapBytes() ?? -1`),
				);
			}
		}
	}

	const errors = page.pageErrors();
	if (errors.length > 0) {
		fail(`page errors during ${subject.name} run: ${errors.join("; ")}`);
	}
	return userAgent;
}

/** `--dist path` names one subject after the app; `--dist name=path` names it. */
function parseSubjects(
	args: CliArguments,
	app: string,
): { name: string; dist: string }[] {
	const raw = args.flags.get("dist");
	if (raw === undefined || raw === true) {
		fail("missing required flag --dist");
	}
	const values = Array.isArray(raw) ? raw : [String(raw)];
	return values.map((value) => {
		const separator = value.indexOf("=");
		if (separator === -1) return { name: app, dist: value };
		return {
			name: value.slice(0, separator),
			dist: value.slice(separator + 1),
		};
	});
}

async function main(): Promise<void> {
	const args: CliArguments = parseArguments(Deno.args);
	const app = requireString(args, "app");
	const runs = optionalNumber(args, "runs") ?? 7;
	const subjectSpecs = parseSubjects(args, app);
	const out = optionalString(args, "out");
	const outDir = optionalString(args, "out-dir");
	if (subjectSpecs.length === 1 && !out && !outDir) {
		fail("pass --out <file> (one subject) or --out-dir <dir>");
	}
	if (subjectSpecs.length > 1 && !outDir) {
		fail("several --dist subjects need --out-dir <dir>");
	}

	const appDir = `${RUNTIME_APPS_ROOT}/${app}`;
	let domScenarios = true;
	try {
		const manifest = JSON.parse(
			await Deno.readTextFile(`${appDir}/bench.app.json`),
		) as { domScenarios?: boolean };
		domScenarios = manifest.domScenarios !== false;
	} catch {
		// no manifest: DOM scenarios available
	}

	const browser = await launchBrowser();
	// One page for every subject: a background tab's animation frames pause,
	// so measuring several pages at once would stall all but the foreground.
	const page = await browser.newPage();
	const subjects: Subject[] = [];
	for (const spec of subjectSpecs) {
		await syncBenchGlue([spec.dist]);
		const server = serveStatic(spec.dist);
		subjects.push({ ...spec, server, metrics: newMetrics() });
		log(`${spec.name}: serving ${spec.dist} at ${server.url}`);
	}
	log(
		`${runs} runs, domScenarios=${domScenarios}, subjects interleaved per run`,
	);

	let userAgent = "";
	try {
		// One full, unrecorded iteration per subject: the first scenarios in a
		// fresh browser pay Chromium's own process and code-path warm-up, which
		// is not a property of the app and would land on whichever subject
		// went first. Loading the page alone did not cover it: the first
		// recorded `clicks` and `table.create-*` samples of the first subject
		// still read several times slower than the rest. Every recorded load is
		// still a cold visit for the app itself — the server sends `no-store`,
		// so nothing is served from cache or code-cached.
		for (const subject of subjects) {
			await measureIteration(
				page,
				{ ...subject, metrics: newMetrics() },
				domScenarios,
			);
		}
		// Subjects alternate within every iteration so machine drift (thermal
		// state, background load) lands on all of them evenly instead of
		// skewing whichever ran last.
		for (let iteration = 0; iteration < runs; iteration += 1) {
			for (const subject of subjects) {
				userAgent = await measureIteration(page, subject, domScenarios);
			}
			log(`run ${iteration + 1}/${runs} complete`);
		}
	} finally {
		await page.close();
		for (const subject of subjects) {
			await subject.server.close();
		}
		await browser.close();
	}

	for (const subject of subjects) {
		const run: BenchmarkRun = aggregate({
			subject: subject.name,
			environment: {
				browser: userAgent,
				platform: `${Deno.build.os} ${Deno.build.arch}`,
				cpus: navigator.hardwareConcurrency,
			},
			runs,
			metrics: subject.metrics,
		});
		const target = outDir ? `${outDir}/${subject.name}-runtime.json` : out!;
		await Deno.writeTextFile(target, JSON.stringify(run, null, "\t") + "\n");
		for (const [id, metric] of Object.entries(run.metrics)) {
			log(`${subject.name} ${id}: ${formatMetric(metric.median, metric.unit)}`);
		}
	}
}

await main();
