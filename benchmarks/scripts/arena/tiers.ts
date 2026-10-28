// Expands `arena/spec/app.json` into the screens of one size tier.
//
// The expansion is deterministic: the same spec and tier always produce the
// same screens, byte for byte, so every framework's emitter writes the same
// app and two runs of the generator never differ. Labels and data are
// distinct per screen, as in a real app, so no framework can share code
// between screens that a real app could not.

import { BENCHMARKS_ROOT } from "../common.ts";

export const TIERS = ["minimal", "medium", "large"] as const;
export type Tier = typeof TIERS[number];

export const SCREEN_KINDS = ["counter", "list", "form", "detail"] as const;
export type ScreenKind = typeof SCREEN_KINDS[number];

/** A screen that counts presses: `Count: n`, and a button adding `step`. */
export interface CounterScreen {
	kind: "counter";
	/** A stable identifier, safe as a file, type, and function name stem. */
	id: string;
	title: string;
	step: number;
}

/** A scroll view of rows, each a title above a subtitle. */
export interface ListScreen {
	kind: "list";
	id: string;
	title: string;
	rows: { title: string; subtitle: string }[];
}

/**
 * Labelled text inputs and a submit button. Submitting checks every field's
 * length against `minLength`: each short field shows
 * `<label> needs at least <minLength> characters`; when every field passes,
 * the screen shows `Saved: <value>, <value>, …` in field order.
 */
export interface FormScreen {
	kind: "form";
	id: string;
	title: string;
	fields: { name: string; label: string; minLength: number }[];
	submitLabel: string;
}

/** A scroll view with the title and paragraphs of text. */
export interface DetailScreen {
	kind: "detail";
	id: string;
	title: string;
	paragraphs: string[];
}

export type Screen = CounterScreen | ListScreen | FormScreen | DetailScreen;

/**
 * One tier's app. With a single screen the app renders it directly; with more,
 * it opens on a home menu (a scroll view with one button per screen, labelled
 * with the screen's title), and every screen has a `Back` button that returns
 * to the menu. Navigation is state holding the current screen's id, with no
 * routing library.
 */
export interface TierApp {
	tier: Tier;
	/** The app title, shown at the top of the home menu. */
	title: string;
	screens: Screen[];
}

interface AppSpec {
	seed: number;
	listRows: number;
	formFields: number;
	detailParagraphs: number;
	tiers: Record<Tier, Record<ScreenKind, number>>;
}

const WORDS = [
	"amber",
	"basin",
	"cedar",
	"delta",
	"ember",
	"fjord",
	"grove",
	"harbor",
	"inlet",
	"juniper",
	"kelp",
	"lagoon",
	"meadow",
	"nectar",
	"orchard",
	"prairie",
	"quarry",
	"ridge",
	"summit",
	"tundra",
	"upland",
	"valley",
	"willow",
	"yarrow",
	"zephyr",
	"beacon",
	"canyon",
	"dune",
	"estuary",
	"glacier",
	"heath",
	"island",
	"marsh",
	"oasis",
	"plateau",
	"reef",
];

/** mulberry32: a small, well-mixed 32-bit generator with a fixed seed. */
function random(seed: number): () => number {
	let state = seed >>> 0;
	return () => {
		state = (state + 0x6d2b79f5) >>> 0;
		let value = state;
		value = Math.imul(value ^ (value >>> 15), value | 1);
		value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
		return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
	};
}

function capitalise(word: string): string {
	return word[0].toUpperCase() + word.slice(1);
}

function phrase(next: () => number, words: number): string {
	const picked: string[] = [];
	for (let index = 0; index < words; index += 1) {
		picked.push(WORDS[Math.floor(next() * WORDS.length)]);
	}
	return picked.join(" ");
}

function sentence(next: () => number, words: number): string {
	const text = phrase(next, words);
	return `${capitalise(text)}.`;
}

export function readSpec(): AppSpec {
	return JSON.parse(
		Deno.readTextFileSync(`${BENCHMARKS_ROOT}/arena/spec/app.json`),
	) as AppSpec;
}

/** Expands one tier. Screens are interleaved by kind: counter, list, form, detail, counter, … */
export function expandTier(tier: Tier, spec: AppSpec = readSpec()): TierApp {
	const counts = spec.tiers[tier];
	// Each tier draws from its own stream so adding screens to one tier never
	// changes another tier's content.
	const next = random(spec.seed ^ TIERS.indexOf(tier) * 0x9e3779b1);
	const screens: Screen[] = [];
	const made: Record<ScreenKind, number> = {
		counter: 0,
		list: 0,
		form: 0,
		detail: 0,
	};

	while (SCREEN_KINDS.some((kind) => made[kind] < counts[kind])) {
		for (const kind of SCREEN_KINDS) {
			if (made[kind] >= counts[kind]) continue;
			made[kind] += 1;
			const number = made[kind];
			const id = `${kind}${String(number).padStart(3, "0")}`;
			const title = `${capitalise(kind)} ${number}: ${
				capitalise(phrase(next, 2))
			}`;
			switch (kind) {
				case "counter":
					screens.push({ kind, id, title, step: 1 + Math.floor(next() * 9) });
					break;
				case "list":
					screens.push({
						kind,
						id,
						title,
						rows: Array.from({ length: spec.listRows }, (_, row) => ({
							title: `${capitalise(phrase(next, 2))} ${row + 1}`,
							subtitle: sentence(next, 6),
						})),
					});
					break;
				case "form":
					screens.push({
						kind,
						id,
						title,
						fields: Array.from({ length: spec.formFields }, (_, field) => {
							const word = phrase(next, 1);
							return {
								name: `${word}${field + 1}`,
								label: `${capitalise(word)} ${field + 1}`,
								minLength: 2 + Math.floor(next() * 6),
							};
						}),
						submitLabel: `Save ${phrase(next, 1)}`,
					});
					break;
				case "detail":
					screens.push({
						kind,
						id,
						title,
						paragraphs: Array.from(
							{ length: spec.detailParagraphs },
							() =>
								Array.from({ length: 4 }, () => sentence(next, 9)).join(" "),
						),
					});
					break;
			}
		}
	}

	return { tier, title: `Arena ${tier}`, screens };
}

/**
 * What a framework provides to generate a tier app: a function that writes
 * the screens and navigator into a copy of the framework's tier project.
 * `projectDir` already holds the framework's committed tier shell; the
 * emitter writes or overwrites only its generated source files.
 */
export interface TierEmitter {
	framework: string;
	emit(app: TierApp, projectDir: string): Promise<void>;
}

/**
 * A double-quoted string literal valid in JavaScript, Dart, and Rust. Tier
 * text is drawn from plain words, so it never holds `$` (interpolation in
 * Dart) or anything JSON escapes differently from those languages.
 */
export function quoted(text: string): string {
	const control = [...text].some((character) => character.charCodeAt(0) < 0x20);
	if (control || /[$\\]/.test(text)) {
		throw new Error(`tier text needs language-specific escaping: ${text}`);
	}
	return JSON.stringify(text);
}
