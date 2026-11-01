// Regenerates the arena tables in `docs/src/benchmarks.md` from the committed
// results: one table per platform measured, and the size-tier table. Only the
// regions between `<!-- BEGIN GENERATED: arena-… -->` markers change.
//
// Usage:
//   deno run -A benchmarks/scripts/arena/docs.ts [--check]

import { fail, log, REPO_ROOT } from "../common.ts";
import { type Platform, PLATFORMS } from "./descriptor.ts";
import type { ArenaResults, ScenarioResult } from "./run.ts";
import { FRAMEWORKS } from "./run.ts";
import type { SizeResults, TierSize } from "./size.ts";
import { type Tier, TIERS } from "./tiers.ts";

const DOCS = `${REPO_ROOT}/docs/src/benchmarks.md`;
const RESULTS = `${REPO_ROOT}/benchmarks/results`;

const NAMES: Record<string, string> = {
	leptoskit: "leptoskit",
	"react-native": "React Native",
	lynx: "Lynx",
	flutter: "Flutter",
};

const PLATFORM_NAMES: Record<Platform, string> = {
	ios: "iOS",
	android: "Android",
	macos: "macOS",
	windows: "Windows",
	linux: "Linux",
	web: "Web",
};

type Unit = "ms" | "us" | "fps" | "hz" | "ms/s" | "per-s";

interface Row {
	label: string;
	scenario: string;
	metric: string;
	unit: Unit;
}

/** The rows each platform table shows, in reading order. */
const ROWS: Row[] = [
	{
		label: "Cold open: launch to ready",
		scenario: "cold",
		metric: "launch_to_ready_ms",
		unit: "ms",
	},
	{
		label: "Big bang: 1,000 cards to the screen",
		scenario: "mount",
		metric: "create_ms",
		unit: "ms",
	},
	{
		label: "Big bang: update every tenth card",
		scenario: "mount",
		metric: "update_ms",
		unit: "ms",
	},
	{
		label: "Big bang: clear 1,000 cards",
		scenario: "mount",
		metric: "clear_ms",
		unit: "ms",
	},
	{
		label: "Echo, async: integer round trip (p50)",
		scenario: "echo",
		metric: "async_int_p50_us",
		unit: "us",
	},
	{
		label: "Echo, async: integer round trip (p99)",
		scenario: "echo",
		metric: "async_int_p99_us",
		unit: "us",
	},
	{
		label: "Echo, async: 1 KiB string (p50)",
		scenario: "echo",
		metric: "async_str1k_p50_us",
		unit: "us",
	},
	{
		label: "Echo, async: 64 KiB of bytes (p50)",
		scenario: "echo",
		metric: "async_bytes64k_p50_us",
		unit: "us",
	},
	{
		label: "Echo, async: 100-field object (p50)",
		scenario: "echo",
		metric: "async_obj_p50_us",
		unit: "us",
	},
	{
		label: "Echo, async: integer calls per second",
		scenario: "echo",
		metric: "async_int_per_s",
		unit: "per-s",
	},
	{
		label: "Echo, sync: integer round trip (p50)",
		scenario: "echo",
		metric: "sync_int_p50_us",
		unit: "us",
	},
	{
		label: "Echo, sync: 64 KiB of bytes (p50)",
		scenario: "echo",
		metric: "sync_bytes64k_p50_us",
		unit: "us",
	},
	{
		label: "Ticker wall: presented frames per second",
		scenario: "ticker",
		metric: "screen_presented_fps",
		unit: "fps",
	},
	{
		label: "Ticker wall: display mode it ran in",
		scenario: "ticker",
		metric: "screen_refresh_hz",
		unit: "hz",
	},
	{
		label: "Ticker wall: hitch time",
		scenario: "ticker",
		metric: "screen_hitch_ratio_ms_per_s",
		unit: "ms/s",
	},
	{
		label: "Ticker wall, tuned: presented frames per second",
		scenario: "ticker.tuned",
		metric: "screen_presented_fps",
		unit: "fps",
	},
	{
		label: "Autoscroll: presented frames per second",
		scenario: "autoscroll",
		metric: "screen_presented_fps",
		unit: "fps",
	},
	{
		label: "Autoscroll: display mode it ran in",
		scenario: "autoscroll",
		metric: "screen_refresh_hz",
		unit: "hz",
	},
	{
		label: "Autoscroll: hitch time",
		scenario: "autoscroll",
		metric: "screen_hitch_ratio_ms_per_s",
		unit: "ms/s",
	},
	{
		label: "Busy hands: presented frames per second",
		scenario: "busy",
		metric: "screen_presented_fps",
		unit: "fps",
	},
	{
		label: "Busy hands: display mode it ran in",
		scenario: "busy",
		metric: "screen_refresh_hz",
		unit: "hz",
	},
	{
		label: "Busy hands: hitch time",
		scenario: "busy",
		metric: "screen_hitch_ratio_ms_per_s",
		unit: "ms/s",
	},
];

function format(value: number, unit: Unit): string {
	switch (unit) {
		case "ms":
			return value >= 100 ? `${value.toFixed(0)} ms` : `${value.toFixed(1)} ms`;
		case "us":
			return value >= 1000
				? `${(value / 1000).toFixed(2)} ms`
				: value >= 10
				? `${value.toFixed(0)} µs`
				: `${value.toFixed(2)} µs`;
		case "fps":
			return value.toFixed(1);
		case "hz":
			return `${value.toFixed(0)} Hz`;
		case "ms/s":
			return `${value.toFixed(1)} ms/s`;
		case "per-s":
			return `${Math.round(value).toLocaleString("en-US")}/s`;
	}
}

export function bytes(value: number): string {
	return value >= 1024 * 1024
		? `${(value / 1024 / 1024).toFixed(2)} MB`
		: `${(value / 1024).toFixed(1)} KB`;
}

/** A results table with footnotes for every cell that holds no number. */
export function renderPlatform(results: ArenaResults): string {
	const frameworks = FRAMEWORKS.filter((framework) =>
		results.frameworks[framework]
	);
	const footnotes: string[] = [];
	const footnote = (text: string): string => {
		let index = footnotes.indexOf(text);
		if (index === -1) index = footnotes.push(text) - 1;
		return ` (${index + 1})`;
	};
	const cell = (
		scenario: ScenarioResult | undefined,
		row: Row,
		framework: string,
	): string | null => {
		if (!scenario) return null;
		if (scenario.status === "unsupported") {
			return `not supported${
				footnote(`${NAMES[framework]}, \`${row.scenario}\`: ${scenario.reason}`)
			}`;
		}
		if (scenario.status === "failed") {
			return `failed${
				footnote(`${NAMES[framework]}, \`${row.scenario}\`: ${scenario.reason}`)
			}`;
		}
		const metric = scenario.metrics[row.metric];
		return metric ? format(metric.median, row.unit) : null;
	};

	const lines: string[] = [];
	const header = ["", ...frameworks.map((framework) => NAMES[framework])];
	lines.push(`| ${header.join(" | ")} |`);
	lines.push(`| ${["---", ...frameworks.map(() => "---:")].join(" | ")} |`);
	for (const row of ROWS) {
		// A row no framework has a number for (a tuned lane nobody has yet,
		// echo on the web) is left out rather than filled with footnotes.
		const measured = frameworks.some((framework) =>
			results.frameworks[framework].scenarios[row.scenario]
				?.metrics[row.metric] !== undefined
		);
		if (!measured) continue;
		const cells = frameworks.map((framework) =>
			cell(
				results.frameworks[framework].scenarios[row.scenario],
				row,
				framework,
			)
		);
		lines.push(
			`| ${[row.label, ...cells.map((value) => value ?? "—")].join(" | ")} |`,
		);
	}

	const kind = results.device.kind;
	const shapeOnly = kind === "simulator" || kind === "emulator" ||
		kind === "browser";
	const versions = frameworks.map((framework) =>
		`${NAMES[framework]} ${results.frameworks[framework].version}`
	).join(", ");
	lines.push("");
	lines.push(
		`_Measured ${
			results.timestamp.slice(0, 10)
		} on ${results.device.name} (${kind}${
			shapeOnly ? ", shape only" : ""
		}), ${results.runs} runs per framework, medians; ${versions}._`,
	);
	for (const framework of frameworks) {
		const caveat = results.frameworks[framework].caveat;
		if (caveat) lines.push("", `_${NAMES[framework]}: ${caveat}_`);
	}
	for (const skipped of results.skipped) {
		lines.push(
			"",
			`_${
				NAMES[skipped.framework] ?? skipped.framework
			} not measured: ${skipped.reason}._`,
		);
	}
	if (footnotes.length > 0) {
		lines.push("");
		for (const [index, text] of footnotes.entries()) {
			lines.push(`${index + 1}. ${text.replaceAll("\n", " ")}`);
		}
	}
	return lines.join("\n");
}

function isSize(
	value: TierSize | { error: string } | undefined,
): value is TierSize {
	return value !== undefined && "bytes" in value;
}

/** Sizes per platform and tier, and the marginal cost of one more screen. */
export function renderSizes(results: SizeResults): string {
	const frameworks = FRAMEWORKS.filter((framework) => results.sizes[framework]);
	const lines: string[] = [];
	lines.push(
		`| platform | tier | ${
			frameworks.map((framework) => NAMES[framework]).join(" | ")
		} |`,
	);
	lines.push(`| --- | --- | ${frameworks.map(() => "---:").join(" | ")} |`);
	const extraScreens = results.screens.large - results.screens.medium;
	for (const platform of PLATFORMS) {
		if (!frameworks.some((framework) => results.sizes[framework][platform])) {
			continue;
		}
		const tierCell = (framework: string, tier: Tier): string => {
			const size = results.sizes[framework][platform]?.[tier];
			if (size === undefined) return "—";
			return isSize(size) ? bytes(size.bytes) : "build failed";
		};
		for (const tier of TIERS) {
			lines.push(
				`| ${PLATFORM_NAMES[platform]} | ${tier} (${results.screens[tier]} ${
					results.screens[tier] === 1 ? "screen" : "screens"
				}) | ${
					frameworks.map((framework) => tierCell(framework, tier)).join(" | ")
				} |`,
			);
		}
		const marginal = frameworks.map((framework) => {
			const [medium, large] = [
				results.sizes[framework][platform]?.medium,
				results.sizes[framework][platform]?.large,
			];
			return isSize(medium) && isSize(large)
				? bytes((large.bytes - medium.bytes) / extraScreens)
				: "—";
		});
		lines.push(
			`| ${PLATFORM_NAMES[platform]} | each extra screen | ${
				marginal.join(" | ")
			} |`,
		);
	}
	lines.push("");
	lines.push(
		`_Built ${
			results.timestamp.slice(0, 10)
		} on ${results.host}. Android: the arm64-v8a release APK. iOS: the unsigned arm64 device \`.app\`. macOS: the \`.app\` bundle. Web: bytes transferred on first load, gzip-compressed as served._`,
	);
	return lines.join("\n");
}

function replaceRegion(document: string, name: string, body: string): string {
	const begin = `<!-- BEGIN GENERATED: ${name} -->`;
	const end = `<!-- END GENERATED: ${name} -->`;
	const start = document.indexOf(begin);
	const stop = document.indexOf(end);
	if (start === -1 || stop === -1) {
		fail(`docs/src/benchmarks.md has no ${name} region`);
	}
	return `${document.slice(0, start + begin.length)}\n\n${body}\n\n${
		document.slice(stop)
	}`;
}

/** The page as the repository's formatter leaves it, so regeneration and `--check` agree with it. */
async function formatted(document: string): Promise<string> {
	const child = new Deno.Command("dprint", {
		args: ["fmt", "--stdin", DOCS],
		cwd: REPO_ROOT,
		stdin: "piped",
		stdout: "piped",
		stderr: "piped",
	}).spawn();
	const writer = child.stdin.getWriter();
	await writer.write(new TextEncoder().encode(document));
	await writer.close();
	const output = await child.output();
	if (!output.success) {
		fail(
			`dprint could not format the page:\n${
				new TextDecoder().decode(output.stderr)
			}`,
		);
	}
	return new TextDecoder().decode(output.stdout);
}

function readJson<T>(path: string): T | null {
	try {
		return JSON.parse(Deno.readTextFileSync(path)) as T;
	} catch {
		return null;
	}
}

async function main(): Promise<void> {
	const check = Deno.args.includes("--check");
	const original = await Deno.readTextFile(DOCS);
	let document = original;
	for (const platform of PLATFORMS) {
		const results = readJson<ArenaResults>(`${RESULTS}/arena-${platform}.json`);
		if (
			!results ||
			!document.includes(`<!-- BEGIN GENERATED: arena-${platform} -->`)
		) continue;
		document = replaceRegion(
			document,
			`arena-${platform}`,
			renderPlatform(results),
		);
	}
	const sizes = readJson<SizeResults>(`${RESULTS}/arena-sizes.json`);
	if (sizes) {
		document = replaceRegion(document, "arena-sizes", renderSizes(sizes));
	}

	document = await formatted(document);
	if (check) {
		if (document !== original) {
			fail("the arena tables are out of date; run benchmark:arena:docs");
		}
		log("arena tables are current");
		return;
	}
	await Deno.writeTextFile(DOCS, document);
	log("regenerated the arena tables");
}

if (import.meta.main) await main();
