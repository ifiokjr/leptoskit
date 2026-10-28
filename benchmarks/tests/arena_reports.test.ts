import { assertEquals, assertThrows } from "jsr:@std/assert@1";
import { type ArenaReport, parseReports } from "../scripts/arena/descriptor.ts";

const report: ArenaReport = {
	v: 1,
	framework: "flutter",
	frameworkVersion: "3.47.8",
	platform: "android",
	scenario: "echo",
	lane: "idiomatic",
	ok: true,
	metrics: { async_int_p50_us: 180 },
};

Deno.test("whole report lines parse wherever the log's prefix ends", () => {
	const log = [
		"10-28 12:00:00.000  1234  1234 I flutter : boot",
		`10-28 12:00:01.000  1234  1234 I ARENA   : ARENA ${
			JSON.stringify(report)
		}`,
	].join("\n");
	assertEquals(parseReports(log), [report]);
});

Deno.test("chunked reports reassemble, and mirrored repeats are dropped", () => {
	const json = JSON.stringify(report);
	const chunks = [json.slice(0, 40), json.slice(40, 90), json.slice(90)];
	const lines = chunks.map((chunk, index) =>
		`I ARENA: ARENA-PART ${index + 1}/3 ${chunk}`
	);
	// The second part arrives twice, as a console mirrored into the system log does.
	const log = [lines[0], lines[1], lines[1], lines[2], `ARENA ${json}`].join(
		"\r\n",
	);
	assertEquals(parseReports(log), [report]);
});

Deno.test("a malformed report is an error, not a skip", () => {
	assertThrows(() => parseReports('ARENA {"v":1,'));
});
