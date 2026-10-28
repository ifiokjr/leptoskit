// What the app needs from the platform it runs on. The web build resolves
// `host.web.ts` instead.

import { Platform } from "react-native";
import ArenaEcho, { type Spec } from "../specs/NativeArenaEcho";

export type HostPlatform = "ios" | "android" | "macos" | "web";

export const hostPlatform: HostPlatform = Platform.OS === "android"
	? "android"
	: Platform.OS === "macos"
	? "macos"
	: "ios";

const { major, minor, patch } = Platform.constants.reactNativeVersion;
export const frameworkVersion = `${major}.${minor}.${patch}`;

/** The native echo module, or null where there is no native layer. */
export const echoModule: Spec | null = ArenaEcho;

/**
 * Writes one line to the system log. Release builds drop `console.log` on
 * iOS, so the line goes through the native module to NSLog (iOS) or logcat
 * with the tag ARENA (Android).
 */
export function writeLine(line: string): void {
	ArenaEcho.log(line);
}
