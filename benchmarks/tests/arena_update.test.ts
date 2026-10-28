import { assertEquals, assertThrows } from "jsr:@std/assert@1";
import {
	compareVersions,
	newestInCaret,
	newestStable,
} from "../scripts/arena/update.ts";

Deno.test("versions compare numerically, not lexically", () => {
	assertEquals(["0.9.0", "0.88.1", "0.10.2"].sort(compareVersions), [
		"0.9.0",
		"0.10.2",
		"0.88.1",
	]);
});

Deno.test("the newest stable version skips pre-releases and nightlies", () => {
	assertEquals(
		newestStable([
			"4.0.3",
			"4.2.0-dev",
			"4.2.0-nightly.202608120616.142.g66ff6ab7",
			"4.1.2",
			"4.2.0-rc.1",
		]),
		"4.1.2",
	);
	assertThrows(() => newestStable(["1.0.0-rc.1"]));
});

Deno.test("a caret range stays within its major version and above its floor", () => {
	const react = [
		"18.3.1",
		"19.2.4",
		"19.3.0",
		"19.3.1",
		"20.0.0",
		"19.4.0-canary",
	];
	assertEquals(newestInCaret(react, "^19.3.0"), "19.3.1");
	assertThrows(() => newestInCaret(react, "^21.0.0"));
});
