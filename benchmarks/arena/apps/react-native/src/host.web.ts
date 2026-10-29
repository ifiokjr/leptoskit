// The web build of `host.ts`: react-native-web renders to the DOM and there
// is no native layer to call.

import rnwPackage from "react-native-web/package.json";
import type { Spec } from "../specs/NativeArenaEcho";

export type HostPlatform = "ios" | "android" | "macos" | "web";

export const hostPlatform: HostPlatform = "web";

/** The react-native-web release, which is what renders on the web. */
export const frameworkVersion: string = rnwPackage.version;

export const echoModule: Spec | null = null;

export function writeLine(line: string): void {
	console.log(line);
}
