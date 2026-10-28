import type { CodegenTypes, TurboModule } from "react-native";
import { TurboModuleRegistry } from "react-native";

/**
 * The arena's echo module, implemented in Objective-C++ (iOS) and Kotlin
 * (Android). Promise methods are the async tier (idiomatic lane); the
 * synchronous methods are the sync tier (tuned lane).
 */
export interface Spec extends TurboModule {
	echo(): Promise<void>;
	addOne(value: number): Promise<number>;
	echoString(value: string): Promise<string>;
	/**
	 * Promise<ArrayBuffer> is unsupported for Objective-C and Kotlin
	 * TurboModules, so the async tier passes bytes as base64 both ways.
	 */
	echoBase64(value: string): Promise<string>;
	echoObject(
		value: CodegenTypes.UnsafeObject,
	): Promise<CodegenTypes.UnsafeObject>;

	/**
	 * Returns null. A void TurboModule method is dispatched to the module's
	 * queue without waiting, so it would not be a round trip.
	 */
	echoSync(): number | null;
	addOneSync(value: number): number;
	echoStringSync(value: string): string;
	echoBytesSync(value: ArrayBuffer): ArrayBuffer;
	echoObjectSync(value: CodegenTypes.UnsafeObject): CodegenTypes.UnsafeObject;

	/** Writes one report line to the system log: NSLog on iOS, logcat tag ARENA on Android. */
	log(line: string): void;
}

export default TurboModuleRegistry.getEnforcing<Spec>("ArenaEcho");
