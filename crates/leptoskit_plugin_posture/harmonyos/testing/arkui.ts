// A scriptable stand-in for the slice of `@kit.ArkUI` the plugin uses, so
// Deno tests can fold a device that does not exist.
import type { common } from "./abilitykit.ts";

interface Rect {
	left: number;
	top: number;
	width: number;
	height: number;
}

type Listener = ((status: number) => void) | ((angles: number[]) => void);

/** The simulated device. Tests set its state, then call `emit*`. */
export const device = {
	foldable: true,
	status: 0,
	creases: [] as Rect[],
	window: { left: 0, top: 0, width: 0, height: 0 } as Rect,
	statusListeners: new Set<(status: number) => void>(),
	angleListeners: new Set<(angles: number[]) => void>(),
	sizeListeners: new Set<() => void>(),
	reset(): void {
		this.foldable = true;
		this.status = 0;
		this.creases = [];
		this.window = { left: 0, top: 0, width: 0, height: 0 };
		this.statusListeners.clear();
		this.angleListeners.clear();
		this.sizeListeners.clear();
	},
	emitStatus(status: number): void {
		this.status = status;
		this.statusListeners.forEach((listener) => listener(status));
	},
	emitAngles(angles: number[]): void {
		this.angleListeners.forEach((listener) => listener(angles));
	},
	emitResize(frame: Rect): void {
		this.window = frame;
		this.sizeListeners.forEach((listener) => listener());
	},
};

// deno-lint-ignore no-namespace
export namespace display {
	export function isFoldable(): boolean {
		return device.foldable;
	}
	export function getFoldStatus(): number {
		return device.status;
	}
	export function getCurrentFoldCreaseRegion(): { creaseRects: Rect[] } {
		return { creaseRects: device.creases };
	}
	export function on(
		type: "foldStatusChange",
		callback: (status: number) => void,
	): void;
	export function on(
		type: "foldAngleChange",
		callback: (angles: number[]) => void,
	): void;
	export function on(type: string, callback: Listener): void {
		if (type === "foldStatusChange") {
			device.statusListeners.add(callback as (status: number) => void);
		} else {
			device.angleListeners.add(callback as (angles: number[]) => void);
		}
	}
	export function off(
		type: "foldStatusChange",
		callback: (status: number) => void,
	): void;
	export function off(
		type: "foldAngleChange",
		callback: (angles: number[]) => void,
	): void;
	export function off(type: string, callback: Listener): void {
		if (type === "foldStatusChange") {
			device.statusListeners.delete(callback as (status: number) => void);
		} else {
			device.angleListeners.delete(callback as (angles: number[]) => void);
		}
	}
}

// deno-lint-ignore no-namespace
export namespace window {
	export interface Window {
		getWindowProperties(): { windowRect: Rect };
		on(type: "windowSizeChange", callback: () => void): void;
		off(type: "windowSizeChange", callback: () => void): void;
	}
	export function getLastWindow(_context: common.Context): Promise<Window> {
		return Promise.resolve({
			getWindowProperties: () => ({ windowRect: device.window }),
			on: (_type: "windowSizeChange", callback: () => void) => {
				device.sizeListeners.add(callback);
			},
			off: (_type: "windowSizeChange", callback: () => void) => {
				device.sizeListeners.delete(callback);
			},
		});
	}
}
