import type { common } from "@kit.AbilityKit";
import { display, window } from "@kit.ArkUI";
import {
	BUSY,
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	NativePlugin,
	PluginError,
} from "../plugins";

// The wire format in leptoskit_plugin_posture's src/lib.rs.
const MAX_HINGES = 4;
const HEADER_BYTES = 12;
const HINGE_BYTES = 36;
const NO_ANGLE = 0xffffffff;
const UNKNOWN = 0;
const CLOSED = 1;
const HALF_OPEN = 2;
const OPEN = 3;
const VERTICAL = 1;
const HORIZONTAL = 2;
const UNKNOWN_FLAG = 0;
const TRUE_FLAG = 2;

/** A rectangle in display pixels, as `@ohos.display` and windows report. */
export interface Rect {
	left: number;
	top: number;
	width: number;
	height: number;
}

/** Everything a posture is built from, sampled at one moment. */
export interface PostureReading {
	foldable: boolean;
	/** `display.FoldStatus`. */
	status: number;
	/** `getCurrentFoldCreaseRegion().creaseRects`, in display pixels. */
	creases: Rect[];
	/** The app window, in display pixels, once known. */
	window: Rect | undefined;
	/** The latest `foldAngleChange` angles, one per hinge. */
	angles: number[];
}

interface Hinge {
	fold: number;
	axis: number;
	/** Fractions of the window, or undefined when the window is not known. */
	region: Rect | undefined;
	angle: number;
}

/**
 * Each hinge's fold for a `display.FoldStatus`, first hinge first. Book folds
 * report 1 expanded, 2 folded, and 3 half folded. API 15 dual-hinge trifolds
 * add two-digit statuses: the ones digit is the first hinge on the same scale,
 * and the tens digit is the second hinge, 1 expanded or 2 half folded. In the
 * one-digit statuses a trifold's second hinge is folded, and it is left out:
 * a folded hinge never crosses the window. Unknown statuses report no hinges.
 */
export function hingeFolds(status: number): number[] {
	const first = [UNKNOWN, OPEN, CLOSED, HALF_OPEN][status % 10] ?? UNKNOWN;
	const second = [UNKNOWN, OPEN, HALF_OPEN][Math.floor(status / 10)];
	if (!Number.isInteger(status) || first === UNKNOWN || second === undefined) {
		return [];
	}
	return second === UNKNOWN ? [first] : [first, second];
}

/** `Posture::fold_of`: the most folded hinge that is not shut. */
function overallFold(folds: number[]): number {
	if (folds.includes(HALF_OPEN)) {
		return HALF_OPEN;
	}
	if (folds.includes(OPEN)) {
		return OPEN;
	}
	return folds.length > 0 && folds.every((fold: number) => fold === CLOSED)
		? CLOSED
		: UNKNOWN;
}

/**
 * Where a crease crosses the window, as fractions of it. A crease outside the
 * window, or one that only touches its edge, does not cross it.
 */
function crossing(crease: Rect, frame: Rect): Rect | undefined {
	const right = frame.left + frame.width;
	const bottom = frame.top + frame.height;
	const left = Math.max(crease.left, frame.left);
	const top = Math.max(crease.top, frame.top);
	const width = Math.min(crease.left + crease.width, right) - left;
	const height = Math.min(crease.top + crease.height, bottom) - top;
	const acrossX = width > 0 || (left > frame.left && left < right);
	const acrossY = height > 0 || (top > frame.top && top < bottom);
	if (
		frame.width <= 0 || frame.height <= 0 || width < 0 || height < 0 ||
		!acrossX || !acrossY
	) {
		return undefined;
	}
	return {
		left: (left - frame.left) / frame.width,
		top: (top - frame.top) / frame.height,
		width: width / frame.width,
		height: height / frame.height,
	};
}

/**
 * Encodes a posture. Creases pair, in order, with the hinges that are not
 * folded, so a trifold showing two panels through its second hinge reports
 * that hinge's state and angle. Every crease that crosses the window is a
 * hinge; a status with no crease (folded, or the window lies beside the fold)
 * still sets the overall fold. `isFoldable()` is false on HarmonyOS flip
 * phones, so false is reported as unknown rather than rigid.
 */
export function encodePosture(reading: PostureReading): ArrayBuffer {
	const folds = hingeFolds(reading.status);
	const open: number[] = [];
	folds.forEach((fold: number, index: number) => {
		if (fold !== CLOSED) {
			open.push(index);
		}
	});
	const hinges: Hinge[] = [];
	reading.creases.forEach((crease: Rect, order: number) => {
		const region = reading.window === undefined
			? undefined
			: crossing(crease, reading.window);
		if (
			hinges.length === MAX_HINGES ||
			(reading.window !== undefined && region === undefined)
		) {
			return;
		}
		const index = open[order];
		const angle = index === undefined ? undefined : reading.angles[index];
		hinges.push({
			fold: index === undefined ? UNKNOWN : folds[index],
			axis: crease.height > crease.width
				? VERTICAL
				: crease.width > crease.height
				? HORIZONTAL
				: UNKNOWN,
			region: region,
			angle: angle === undefined || !Number.isFinite(angle)
				? NO_ANGLE
				: Math.min(Math.max(Math.round(angle), 0), 360),
		});
	});
	const output = new ArrayBuffer(HEADER_BYTES + HINGE_BYTES * hinges.length);
	const view = new DataView(output);
	view.setUint32(0, reading.foldable ? TRUE_FLAG : UNKNOWN_FLAG, true);
	view.setUint32(4, overallFold(folds), true);
	view.setUint32(8, hinges.length, true);
	hinges.forEach((hinge: Hinge, index: number) => {
		const offset = HEADER_BYTES + HINGE_BYTES * index;
		const region = hinge.region;
		view.setUint32(offset, hinge.fold, true);
		view.setUint32(offset + 4, hinge.axis, true);
		view.setUint32(offset + 8, UNKNOWN_FLAG, true);
		view.setUint32(offset + 12, region === undefined ? 0 : 1, true);
		[
			region?.left ?? 0,
			region?.top ?? 0,
			region?.width ?? 0,
			region?.height ?? 0,
		]
			.forEach((value: number, field: number) => {
				view.setFloat32(
					offset + 16 + 4 * field,
					Math.min(Math.max(value, 0), 1),
					true,
				);
			});
		view.setUint32(offset + 32, hinge.angle, true);
	});
	return output;
}

function same(left: ArrayBuffer, right: ArrayBuffer): boolean {
	if (left.byteLength !== right.byteLength) {
		return false;
	}
	const a = new Uint8Array(left);
	const b = new Uint8Array(right);
	return a.every((byte: number, index: number) => byte === b[index]);
}

/**
 * Fold state from `@ohos.display`: the fold status for each hinge's state,
 * crease regions for where each hinge crosses the window, and fold angles
 * (API 12) for each hinge's angle. Subscriptions start with the first call
 * and last until the guest unloads, because fold angles arrive only as
 * events and a snapshot needs the latest ones.
 */
export class PosturePlugin implements NativePlugin {
	readonly name = "posture";
	readonly methods = [0, 1];
	private context: common.Context;
	private target: Promise<window.Window> | undefined = undefined;
	private frame: Rect | undefined = undefined;
	private angles: number[] = [];
	private waiting = false;
	private changed: (() => void) | undefined = undefined;
	private stops: (() => void)[] = [];
	private closed = false;

	constructor(context: common.Context) {
		this.context = context;
	}

	async invoke(call: NativeCall): Promise<ArrayBuffer> {
		if (this.closed) {
			throw new PluginError(CANCELLED);
		}
		if (call.method === 0) {
			if (call.payload.byteLength !== 0) {
				throw new PluginError(INVALID_ARGUMENT);
			}
			await this.observe();
			return this.encode();
		}
		const length = call.payload.byteLength;
		if (
			length < HEADER_BYTES || (length - HEADER_BYTES) % HINGE_BYTES !== 0 ||
			(length - HEADER_BYTES) / HINGE_BYTES > MAX_HINGES
		) {
			throw new PluginError(INVALID_ARGUMENT);
		}
		if (this.waiting) {
			throw new PluginError(BUSY);
		}
		this.waiting = true;
		try {
			await this.observe();
			return await this.next(call);
		} finally {
			this.waiting = false;
			this.changed = undefined;
		}
	}

	close(): void {
		this.closed = true;
		const stops = this.stops;
		this.stops = [];
		stops.forEach((stop: () => void) => stop());
	}

	/** Resolves with the first posture that differs from the call's payload. */
	private next(call: NativeCall): Promise<ArrayBuffer> {
		return new Promise<ArrayBuffer>((resolve, reject) => {
			const check = (): void => {
				if (this.closed) {
					reject(new PluginError(CANCELLED));
					return;
				}
				// Runs inside platform callbacks, so failures go to the guest.
				try {
					const current = this.encode();
					if (!same(current, call.payload)) {
						resolve(current);
					}
				} catch (error) {
					reject(error);
				}
			};
			call.onCancel(() => reject(new PluginError(CANCELLED)));
			this.changed = check;
			// A change between the guest's last posture and this call is
			// reported immediately.
			check();
		});
	}

	private encode(): ArrayBuffer {
		const foldable = display.isFoldable();
		return encodePosture({
			foldable: foldable,
			status: foldable ? display.getFoldStatus() : 0,
			creases: foldable ? display.getCurrentFoldCreaseRegion().creaseRects : [],
			window: this.frame,
			angles: this.angles,
		});
	}

	private notify(): void {
		this.changed?.();
	}

	/** Starts the fold, angle, and window subscriptions once. */
	private async observe(): Promise<void> {
		if (this.target !== undefined || !display.isFoldable()) {
			await this.target;
			return;
		}
		this.target = window.getLastWindow(this.context);
		const status = (): void => this.notify();
		display.on("foldStatusChange", status);
		this.stops.push(() => display.off("foldStatusChange", status));
		const angles = (values: number[]): void => {
			this.angles = values;
			this.notify();
		};
		display.on("foldAngleChange", angles);
		this.stops.push(() => display.off("foldAngleChange", angles));
		const target = await this.target;
		if (this.closed) {
			return;
		}
		const resize = (): void => {
			this.frame = target.getWindowProperties().windowRect;
			this.notify();
		};
		this.frame = target.getWindowProperties().windowRect;
		target.on("windowSizeChange", resize);
		this.stops.push(() => target.off("windowSizeChange", resize));
	}
}
