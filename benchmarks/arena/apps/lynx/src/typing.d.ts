// What the native hosts provide to the bundle: launch data through
// `lynx.__globalProps`, and two native modules registered by the iOS and
// Android hosts. The web host provides only the global props.

/** A JSON object as the native module bridges carry it. */
export type EchoObject = Record<string, number>;

/**
 * The echo module, written in the platform language (`ArenaEchoModule.m` on
 * iOS, `ArenaEchoModule.kt` on Android). Each payload has two methods: the
 * callback form, which Lynx delivers back to the background thread as a
 * separate task (the async tier), and the returning form, which Lynx runs on
 * the background thread and returns from directly (the sync tier).
 */
export interface ArenaEchoModule {
	echo(callback: () => void): void;
	addOne(value: number, callback: (result: number) => void): void;
	echoString(value: string, callback: (result: string) => void): void;
	echoBytes(value: ArrayBuffer, callback: (result: ArrayBuffer) => void): void;
	echoObject(value: EchoObject, callback: (result: EchoObject) => void): void;

	echoSync(): void;
	addOneSync(value: number): number;
	echoStringSync(value: string): string;
	echoBytesSync(value: ArrayBuffer): ArrayBuffer;
	echoObjectSync(value: EchoObject): EchoObject;
}

/** Writes one line to the platform log: `NSLog` on iOS, logcat tag `ARENA` on Android. */
export interface ArenaLogModule {
	log(line: string): void;
}

declare module "@lynx-js/types" {
	interface GlobalProps {
		/** The scenario to run; absent for the home screen. */
		scenario?: string;
		/** `tuned` selects the escape-hatch lane where a scenario has one. */
		lane?: string;
		platform?: string;
		/** The Lynx engine release the host links (web: `@lynx-js/web-core`). */
		frameworkVersion?: string;
		/** The display's refresh rate in Hz, for counting late frame callbacks. */
		refreshRate?: number;
		/** Process start as a Unix timestamp in milliseconds, when the host knows it. */
		processStartUnixMs?: number;
	}

	interface NativeModules {
		ArenaEcho?: ArenaEchoModule;
		ArenaLog?: ArenaLogModule;
	}
}
