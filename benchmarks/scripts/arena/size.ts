// The arena's size tiers: the same minimal, medium, and large app, generated
// for every framework from `arena/spec/app.json`, built in release, and
// measured as each platform ships it.
//
// Usage:
//   deno run -A benchmarks/scripts/arena/size.ts \
//     [--frameworks leptoskit,flutter,...] [--platforms android,ios,macos,web] \
//     [--tiers minimal,medium,large] [--out benchmarks/results/arena-sizes.json]
//
// Each tier project is a fresh copy of the framework's committed tier shell
// under `target/arena/tiers/<framework>/<tier>`, with the emitter's screens
// written into it. Builds that fail are recorded with their error, never
// left out silently.

import { launchBrowser } from "../cdp.ts";
import {
	currentCommit,
	fail,
	log,
	optionalString,
	parseArguments,
	REPO_ROOT,
} from "../common.ts";
import { serveStatic } from "../serve.ts";
import {
	ARENA_APPS_ROOT,
	type Platform,
	PLATFORMS,
	readDescriptor,
} from "./descriptor.ts";
import { FRAMEWORKS, shell } from "./run.ts";
import { expandTier, type Tier, type TierEmitter, TIERS } from "./tiers.ts";

export interface TierSize {
	/** What the platform downloads: the APK, the bundle, or the web transfer. */
	bytes: number;
	/** The uncompressed native libraries inside an APK. */
	nativeLibraryBytes?: number;
}

export interface SizeResults {
	schema: 1;
	subject: "arena-sizes";
	timestamp: string;
	gitCommit: string | null;
	host: string;
	/** framework → platform → tier → size, or the reason it has none. */
	sizes: Record<
		string,
		Partial<
			Record<Platform, Partial<Record<Tier, TierSize | { error: string }>>>
		>
	>;
	/** Screens per tier, so the marginal cost of a screen can be derived. */
	screens: Record<Tier, number>;
}

function list<T extends string>(
	value: string | undefined,
	all: readonly T[],
): T[] {
	if (!value) return [...all];
	const picked = value.split(",").map((item) => item.trim()) as T[];
	for (const item of picked) {
		if (!all.includes(item)) {
			fail(`unknown value ${item}; expected one of ${all.join(", ")}`);
		}
	}
	return picked;
}

function treeBytes(path: string): number {
	const info = Deno.statSync(path);
	if (info.isFile) return info.size;
	let total = 0;
	for (const entry of Deno.readDirSync(path)) {
		if (entry.isSymlink) continue;
		total += treeBytes(`${path}/${entry.name}`);
	}
	return total;
}

/** The uncompressed size of an APK's `lib/` entries, from `unzip -l`. */
function apkNativeLibraries(apk: string): number {
	const listing = new TextDecoder().decode(
		new Deno.Command("unzip", { args: ["-l", apk] }).outputSync().stdout,
	);
	let total = 0;
	for (const line of listing.split("\n")) {
		const match = line.match(/^\s*(\d+)\s+\S+\s+\S+\s+(lib\/\S+\.so)$/);
		if (match) total += Number(match[1]);
	}
	return total;
}

/** Bytes a browser transfers loading the app, served gzip-compressed, until the network is idle. */
async function webTransfer(dist: string): Promise<number> {
	const browser = await launchBrowser();
	const server = serveStatic(dist);
	const page = await browser.newPage();
	try {
		await page.navigate(`${server.url}/`);
		let last = -1;
		while (last !== page.networkBytes()) {
			last = page.networkBytes();
			await new Promise((resolve) => setTimeout(resolve, 2000));
		}
		return last;
	} finally {
		await page.close();
		await server.close();
		await browser.close();
	}
}

async function measure(
	platform: Platform,
	artifact: string,
): Promise<TierSize> {
	if (platform === "web") return { bytes: await webTransfer(artifact) };
	if (artifact.endsWith(".apk")) {
		return {
			bytes: Deno.statSync(artifact).size,
			nativeLibraryBytes: apkNativeLibraries(artifact),
		};
	}
	return { bytes: treeBytes(artifact) };
}

async function main(): Promise<void> {
	const args = parseArguments(Deno.args);
	const frameworks = list(optionalString(args, "frameworks"), FRAMEWORKS);
	const platforms = list(optionalString(args, "platforms"), PLATFORMS);
	const tiers = list(optionalString(args, "tiers"), TIERS);
	const out = optionalString(args, "out") ??
		`${REPO_ROOT}/benchmarks/results/arena-sizes.json`;

	// Platforms are measured on different machines (Windows and Linux in CI,
	// Apple platforms on a Mac), so a run replaces only the sizes it measured.
	let previous: SizeResults["sizes"] = {};
	try {
		previous = (JSON.parse(await Deno.readTextFile(out)) as SizeResults).sizes;
	} catch {
		// First run, or no results committed yet.
	}
	const results: SizeResults = {
		schema: 1,
		subject: "arena-sizes",
		timestamp: new Date().toISOString(),
		gitCommit: currentCommit(),
		host: `${Deno.build.os} ${Deno.build.arch}`,
		sizes: previous,
		screens: Object.fromEntries(
			TIERS.map((tier) => [tier, expandTier(tier).screens.length]),
		) as Record<Tier, number>,
	};

	for (const framework of frameworks) {
		let descriptor;
		try {
			descriptor = readDescriptor(framework);
		} catch {
			log(`skipping ${framework}: no arena.json yet`);
			continue;
		}
		const module = await import(`./${descriptor.tiers.emitter}`) as {
			emitter: TierEmitter;
		};
		const shellDir =
			`${ARENA_APPS_ROOT}/${framework}/${descriptor.tiers.shell}`;
		const perPlatform: SizeResults["sizes"][string] =
			results.sizes[framework] ?? {};
		results.sizes[framework] = perPlatform;

		for (const tier of tiers) {
			const project = `${REPO_ROOT}/target/arena/tiers/${framework}/${tier}`;
			await Deno.remove(project, { recursive: true }).catch(() => {});
			await Deno.mkdir(project, { recursive: true });
			// Build outputs never travel with the shell; dependency caches do,
			// so ten tier builds don't download the same packages ten times.
			// The excludes are anchored to the shell's own build directories:
			// unanchored, `dist` and `build` would also strip packages inside
			// node_modules.
			const outputs = [
				"/build",
				"/dist",
				"/target",
				"/android/app/build",
				"/android/build",
				"/ios/build",
			];
			await shell(
				`rsync -a ${
					outputs.map((path) => `--exclude ${path}`).join(" ")
				} "${shellDir}/" "${project}/"`,
				REPO_ROOT,
			);
			await module.emitter.emit(expandTier(tier), project);

			for (const platform of platforms) {
				const build = descriptor.tiers.platforms[platform];
				if (!build) continue;
				const slot = perPlatform[platform] ?? {};
				perPlatform[platform] = slot;
				try {
					await shell(build.build, project, project);
					const size = await measure(platform, `${project}/${build.artifact}`);
					slot[tier] = size;
					log(`${framework} ${tier} ${platform}: ${size.bytes} bytes`);
				} catch (error) {
					slot[tier] = { error: (error as Error).message };
				}
			}
		}
	}

	await Deno.writeTextFile(out, `${JSON.stringify(results, null, "\t")}\n`);
	log(`wrote ${out}`);
}

if (import.meta.main) await main();
