// Moves the arena to the latest stable release of every framework: resolves
// versions from each project's own registry (npm, Maven Central, Lynx's pod
// spec repository, Flutter's release feed), rewrites `arena/versions.json`,
// and runs each app's `update` command to sync its manifests and lockfiles.
//
// Usage:
//   deno run -A benchmarks/scripts/arena/update.ts [--frameworks …] [--dry-run]
//
// Registries are asked anonymously, with no headers beyond the runtime's
// defaults. Pre-releases (anything with a suffix: -rc, -dev, nightlies) are
// never picked.

import { fail, log, optionalString, parseArguments } from "../common.ts";
import { ARENA_APPS_ROOT, ARENA_ROOT, readDescriptor } from "./descriptor.ts";
import { FRAMEWORKS, shell } from "./run.ts";

type Versions = Record<string, string>;

const STABLE = /^\d+\.\d+\.\d+$/;

/** Orders stable semver strings ascending. */
export function compareVersions(left: string, right: string): number {
	const [a, b] = [left.split(".").map(Number), right.split(".").map(Number)];
	for (let index = 0; index < 3; index += 1) {
		if (a[index] !== b[index]) return a[index] - b[index];
	}
	return 0;
}

/** The newest stable version in a list, ignoring pre-releases. */
export function newestStable(versions: string[]): string {
	const stable = versions.filter((version) => STABLE.test(version)).sort(
		compareVersions,
	);
	const newest = stable.at(-1);
	if (!newest) {
		throw new Error(`no stable version among ${versions.slice(-5).join(", ")}`);
	}
	return newest;
}

/** The newest stable version satisfying a caret range such as `^19.3.0`. */
export function newestInCaret(versions: string[], range: string): string {
	const floor = range.replace(/^\^/, "");
	if (!STABLE.test(floor)) throw new Error(`unsupported range ${range}`);
	const [major] = floor.split(".").map(Number);
	return newestStable(
		versions.filter((version) =>
			STABLE.test(version) && Number(version.split(".")[0]) === major &&
			compareVersions(version, floor) >= 0
		),
	);
}

async function json<T>(url: string): Promise<T> {
	const response = await fetch(url);
	if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
	return await response.json() as T;
}

async function npm(
	name: string,
): Promise<
	{
		versions: string[];
		latest: string;
		peers: (version: string) => Record<string, string>;
	}
> {
	const document = await json<{
		"dist-tags": { latest: string };
		versions: Record<string, { peerDependencies?: Record<string, string> }>;
	}>(`https://registry.npmjs.org/${name.replace("/", "%2F")}`);
	return {
		versions: Object.keys(document.versions),
		latest: document["dist-tags"].latest,
		peers: (version) => document.versions[version]?.peerDependencies ?? {},
	};
}

async function npmLatest(name: string): Promise<string> {
	const { latest } = await npm(name);
	if (!STABLE.test(latest)) {
		throw new Error(`${name}'s latest tag is a pre-release: ${latest}`);
	}
	return latest;
}

async function maven(group: string, artifact: string): Promise<string> {
	const response = await fetch(
		`https://repo1.maven.org/maven2/${
			group.replaceAll(".", "/")
		}/${artifact}/maven-metadata.xml`,
	);
	if (!response.ok) {
		throw new Error(`maven ${group}:${artifact}: HTTP ${response.status}`);
	}
	const xml = await response.text();
	return newestStable(
		[...xml.matchAll(/<version>([^<]+)<\/version>/g)].map((match) => match[1]),
	);
}

/**
 * Lynx publishes its iOS pods to its own spec repository, not CocoaPods trunk
 * (trunk stops at 4.0.3): one directory per version under `Lynx/`.
 */
async function lynxPod(): Promise<string> {
	const entries = await json<{ name: string }[]>(
		"https://api.github.com/repos/lynx-family/Specs/contents/Lynx",
	);
	return newestStable(entries.map((entry) => entry.name));
}

async function flutterStable(): Promise<string> {
	const document = await json<{
		current_release: { stable: string };
		releases: { hash: string; channel: string; version: string }[];
	}>(
		"https://storage.googleapis.com/flutter_infra_release/releases/releases_macos.json",
	);
	const release = document.releases.find((entry) =>
		entry.hash === document.current_release.stable && entry.channel === "stable"
	);
	if (!release) {
		throw new Error("Flutter's releases list has no current stable release");
	}
	return release.version;
}

/** Resolves every version the arena pins, from the registries each framework publishes to. */
export async function resolveVersions(): Promise<Versions> {
	const reactNative = await npmLatest("react-native");
	const react = await npm("react");
	const reactRange = (await npm("react-native")).peers(reactNative).react;
	return {
		"react-native": reactNative,
		react: reactRange
			? newestInCaret(react.versions, reactRange.split("||")[0].trim())
			: react.latest,
		"react-native-web": await npmLatest("react-native-web"),
		"react-native-macos": await npmLatest("react-native-macos"),
		"react-native-windows": await npmLatest("react-native-windows"),
		"@lynx-js/react": await npmLatest("@lynx-js/react"),
		"@lynx-js/rspeedy": await npmLatest("@lynx-js/rspeedy"),
		"@lynx-js/react-rsbuild-plugin": await npmLatest(
			"@lynx-js/react-rsbuild-plugin",
		),
		"@lynx-js/web-core": await npmLatest("@lynx-js/web-core"),
		"@lynx-js/web-elements": await npmLatest("@lynx-js/web-elements"),
		"lynx-android": await maven("org.lynxsdk.lynx", "lynx"),
		"lynx-ios": await lynxPod(),
		flutter: await flutterStable(),
	};
}

async function main(): Promise<void> {
	const args = parseArguments(Deno.args);
	const dryRun = args.flags.get("dry-run") === true;
	const frameworks =
		(optionalString(args, "frameworks")?.split(",") ?? [...FRAMEWORKS]).map((
			name,
		) => name.trim());
	const path = `${ARENA_ROOT}/versions.json`;
	const current = JSON.parse(await Deno.readTextFile(path)) as Versions;

	const resolved = await resolveVersions();
	const changes = Object.entries(resolved).filter(([name, version]) =>
		current[name] !== version
	);
	for (const [name, version] of changes) {
		log(`${name}: ${current[name] ?? "none"} → ${version}`);
	}
	if (changes.length === 0) {
		log("every framework is already at its latest stable release");
	}
	if (dryRun) return;

	const next: Versions = {
		$comment: current.$comment,
		resolved: new Date().toISOString().slice(0, 10),
		leptoskit: "workspace",
		...resolved,
	};
	await Deno.writeTextFile(path, `${JSON.stringify(next, null, "\t")}\n`);

	for (const framework of frameworks) {
		let update: string;
		try {
			update = readDescriptor(framework).update;
		} catch {
			log(`skipping ${framework}: no arena.json yet`);
			continue;
		}
		try {
			await shell(update, `${ARENA_APPS_ROOT}/${framework}`);
		} catch (error) {
			fail(
				`${framework} could not move to the new versions: ${
					(error as Error).message
				}`,
			);
		}
	}
}

if (import.meta.main) await main();
