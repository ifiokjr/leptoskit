import { assertEquals } from "jsr:@std/assert@1";
import { mergeResults, xcodeEnvironment } from "../scripts/arena/run.ts";

Deno.test("Xcode builds drop the nix toolchain and put Xcode's tools first", () => {
	const environment = xcodeEnvironment({
		PATH:
			"/nix/store/abc-devenv-profile/bin:/Users/me/.cargo/bin:/usr/bin:/bin",
		CC: "clang",
		LD: "ld",
		NIX_CFLAGS_COMPILE: "-isystem /nix/store/x/include",
		MACOSX_DEPLOYMENT_TARGET: "14.0",
		DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
		HOME: "/Users/me",
	});
	assertEquals(environment, {
		PATH:
			"/usr/bin:/Users/me/.cargo/bin:/bin:/nix/store/abc-devenv-profile/bin",
		DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
		HOME: "/Users/me",
	});
});

Deno.test("a partial run replaces only what it measured on the same device", () => {
	const device = {
		platform: "android" as const,
		name: "Seeker",
		kind: "physical" as const,
		refreshHz: 120,
		refreshRatesHz: [60, 120],
	};
	const base = {
		schema: 1 as const,
		subject: "arena-android",
		timestamp: "",
		gitCommit: null,
		host: { platform: "", cpus: 1, load: 0 },
		runs: 5,
	};
	const scenario = (median: number) => ({
		status: "ok" as const,
		notes: [],
		metrics: {
			x: { values: [median], median, p95: median, min: median, max: median },
		},
	});
	const previous = {
		...base,
		device,
		frameworks: {
			leptoskit: {
				version: "0.1.0",
				scenarios: { cold: scenario(1), mount: scenario(2) },
			},
			flutter: { version: "3.47.8", scenarios: { cold: scenario(3) } },
		},
		skipped: [],
	};
	const next = {
		...base,
		device,
		frameworks: {
			leptoskit: { version: "0.1.0", scenarios: { mount: scenario(9) } },
		},
		skipped: [],
	};
	const merged = mergeResults(previous, next);
	assertEquals(merged.frameworks.leptoskit.scenarios.cold.metrics.x.median, 1);
	assertEquals(merged.frameworks.leptoskit.scenarios.mount.metrics.x.median, 9);
	assertEquals(merged.frameworks.flutter.scenarios.cold.metrics.x.median, 3);
	assertEquals(
		mergeResults(previous, { ...next, device: { ...device, name: "Pixel" } }),
		{ ...next, device: { ...device, name: "Pixel" } },
	);
});
