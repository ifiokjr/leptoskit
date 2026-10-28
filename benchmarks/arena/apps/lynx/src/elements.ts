// Main-thread helpers for reading what the element tree actually shows.

import type { MainThread } from "@lynx-js/types";

/**
 * The text a `<text>` element renders. ReactLynx sets a lone string child as
 * the element's `text` attribute; anything else lives in `raw-text` children.
 */
export function textOf(element: MainThread.Element | undefined | null): string {
	"main thread";
	if (element === undefined || element === null) return "";
	const direct: unknown = element.getAttribute("text");
	if (typeof direct === "string") return direct;
	const raw = element.querySelector("raw-text");
	const nested: unknown = raw === null ? undefined : raw.getAttribute("text");
	return typeof nested === "string" ? nested : "";
}
