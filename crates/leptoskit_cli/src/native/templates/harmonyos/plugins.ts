// ArkTS plugins for the HarmonyOS host. The generated registrant constructs
// each selected plugin; the page owns one dispatcher per guest runtime. Every
// call runs on the UI thread, and every completion is delivered on a later
// turn, so a plugin never re-enters the runtime while it renders.

/** Status codes from `leptoskit_bridge::Error`. */
export const UNAVAILABLE = 1;
export const PERMISSION_DENIED = 2;
export const INVALID_ARGUMENT = 3;
export const BUSY = 4;
export const TOO_LARGE = 5;
export const CANCELLED = 6;
export const NATIVE = 7;

/** A failure the guest receives as its bridge error. */
export class PluginError extends Error {
	readonly code: number;

	constructor(code: number) {
		super(`Native plugin error ${code}`);
		this.code = code;
	}
}

/**
 * One guest call. The guest cancels it by dropping its future; the host
 * cancels every call when the runtime closes. A cancelled call's result is
 * discarded, so plugins stop observers in `onCancel` rather than settling.
 */
export class NativeCall {
	readonly method: number;
	readonly payload: ArrayBuffer;
	private isCancelled = false;
	private handlers: (() => void)[] = [];

	constructor(method: number, payload: ArrayBuffer) {
		this.method = method;
		this.payload = payload;
	}

	get cancelled(): boolean {
		return this.isCancelled;
	}

	/** Runs `handler` once on cancellation, or now if already cancelled. */
	onCancel(handler: () => void): void {
		if (this.isCancelled) {
			handler();
		} else {
			this.handlers.push(handler);
		}
	}

	/** Called by the dispatcher; plugins never cancel their own calls. */
	cancel(): void {
		if (this.isCancelled) {
			return;
		}

		this.isCancelled = true;
		const handlers = this.handlers;
		this.handlers = [];
		handlers.forEach((handler: () => void) => handler());
	}
}

/** A platform plugin implemented in ArkTS. */
export interface NativePlugin {
	readonly name: string;
	readonly methods: number[];
	/** Resolve with the reply bytes, or reject with a `PluginError`. */
	invoke(call: NativeCall): Promise<ArrayBuffer>;
	/** Stop subscriptions before the guest unloads. */
	close(): void;
}

/** Delivers a settled call back to `Runtime.complete`. */
export type Completion = (
	id: number,
	code: number,
	payload: ArrayBuffer,
) => void;

/**
 * Routes guest requests to ArkTS plugins and their replies back. The page
 * passes each `RenderBatch`'s `cancelled` IDs to `cancel` before its
 * `requests` to `start`, so the dispatcher never sees `Runtime`'s types and
 * ArkTS pages need no structural typing.
 */
export class NativePlugins {
	private plugins: Map<number, NativePlugin> = new Map();
	private calls: Map<number, NativeCall> = new Map();
	private complete: Completion;
	private closed = false;

	constructor(complete: Completion) {
		this.complete = complete;
	}

	/** Adds a plugin under the ID `Runtime.register` returned for it. */
	add(id: number, plugin: NativePlugin): void {
		if (this.plugins.has(id)) {
			throw new Error(`Duplicate native plugin ${plugin.name}`);
		}

		this.plugins.set(id, plugin);
	}

	/** Cancels a call the guest no longer awaits; its result is discarded. */
	cancel(id: number): void {
		const call = this.calls.get(id);

		if (call !== undefined) {
			this.calls.delete(id);
			call.cancel();
		}
	}

	/** Cancels outstanding calls and closes every plugin. */
	close(): void {
		if (this.closed) {
			return;
		}

		this.closed = true;
		const calls = Array.from(this.calls.values());
		this.calls.clear();
		calls.forEach((call: NativeCall) => call.cancel());
		const plugins = Array.from(this.plugins.values());
		this.plugins.clear();
		let failure: Error | undefined = undefined;

		for (const plugin of plugins) {
			try {
				plugin.close();
			} catch (error) {
				failure = failure ?? (error as Error);
			}
		}

		if (failure !== undefined) {
			throw failure;
		}
	}

	/** Invokes a plugin. Unknown plugins and methods fail as unavailable. */
	start(
		id: number,
		plugin: number,
		method: number,
		payload: ArrayBuffer,
	): void {
		if (this.closed) {
			return;
		}

		const call = new NativeCall(method, payload);
		this.calls.set(id, call);
		const handler = this.plugins.get(plugin);
		let reply: Promise<ArrayBuffer>;
		try {
			reply = handler !== undefined && handler.methods.includes(method)
				? handler.invoke(call)
				: Promise.reject(new PluginError(UNAVAILABLE));
		} catch (error) {
			reply = Promise.reject(error);
		}
		reply.then(
			(bytes: ArrayBuffer) => this.settle(id, call, 0, bytes),
			(error: Error) => {
				if (!(error instanceof PluginError)) {
					console.error(`LeptosKit native plugin call failed: ${error}`);
				}

				const code = error instanceof PluginError ? error.code : NATIVE;
				this.settle(id, call, code, new ArrayBuffer(0));
			},
		);
	}

	private settle(
		id: number,
		call: NativeCall,
		code: number,
		payload: ArrayBuffer,
	): void {
		if (this.calls.get(id) !== call) {
			return;
		}

		this.calls.delete(id);
		this.complete(id, code, payload);
	}
}
