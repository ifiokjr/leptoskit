import { deepStrictEqual, equal, rejects } from "node:assert/strict";
import {
	BUSY,
	CANCELLED,
	INVALID_ARGUMENT,
	NativeCall,
	PluginError,
} from "../plugins";
import {
	encodePosture,
	hingeFolds,
	PosturePlugin,
	PostureReading,
} from "./PosturePlugin.ts";
import { device } from "./testing/arkui.ts";

const [UNKNOWN, CLOSED, HALF_OPEN, OPEN] = [0, 1, 2, 3];
const [VERTICAL, HORIZONTAL] = [1, 2];

interface DecodedHinge {
	fold: number;
	axis: number;
	occluding: number;
	region: number[] | undefined;
	angle: number | undefined;
}

/** Mirrors `Posture::decode`, rounding regions to the f32 precision sent. */
function decode(bytes: ArrayBuffer) {
	const view = new DataView(bytes);
	const count = view.getUint32(8, true);
	equal(bytes.byteLength, 12 + 36 * count);
	const hinges: DecodedHinge[] = [];
	for (let index = 0; index < count; index++) {
		const at = 12 + 36 * index;
		const word = (offset: number) => view.getUint32(at + offset, true);
		const region = [16, 20, 24, 28].map((offset) =>
			Math.round(view.getFloat32(at + offset, true) * 1000) / 1000
		);
		hinges.push({
			fold: word(0),
			axis: word(4),
			occluding: word(8),
			region: word(12) === 1 ? region : undefined,
			angle: word(32) === 0xffffffff ? undefined : word(32),
		});
	}
	return {
		foldable: view.getUint32(0, true),
		fold: view.getUint32(4, true),
		hinges: hinges,
	};
}

function reading(overrides: Partial<PostureReading>): PostureReading {
	return {
		foldable: true,
		status: 0,
		creases: [],
		window: { left: 0, top: 0, width: 2000, height: 1000 },
		angles: [],
		...overrides,
	};
}

Deno.test("fold statuses map to each hinge, including API 15 trifolds", () => {
	deepStrictEqual(hingeFolds(0), []);
	deepStrictEqual(hingeFolds(1), [OPEN]);
	deepStrictEqual(hingeFolds(2), [CLOSED]);
	deepStrictEqual(hingeFolds(3), [HALF_OPEN]);
	deepStrictEqual(hingeFolds(11), [OPEN, OPEN]);
	deepStrictEqual(hingeFolds(12), [CLOSED, OPEN]);
	deepStrictEqual(hingeFolds(13), [HALF_OPEN, OPEN]);
	deepStrictEqual(hingeFolds(21), [OPEN, HALF_OPEN]);
	deepStrictEqual(hingeFolds(22), [CLOSED, HALF_OPEN]);
	deepStrictEqual(hingeFolds(23), [HALF_OPEN, HALF_OPEN]);
	for (const unknown of [-1, 4, 10, 14, 31, 1.5, Number.NaN]) {
		deepStrictEqual(hingeFolds(unknown), [], `status ${unknown}`);
	}
});

Deno.test("a half-open book fold reports its crease as window fractions", () => {
	const book = decode(encodePosture(reading({
		status: 3,
		creases: [{ left: 980, top: 0, width: 40, height: 1000 }],
		angles: [97.4],
	})));
	deepStrictEqual(book, {
		foldable: 2,
		fold: HALF_OPEN,
		hinges: [{
			fold: HALF_OPEN,
			axis: VERTICAL,
			occluding: 0,
			region: [0.49, 0, 0.02, 1],
			angle: 97,
		}],
	});
});

Deno.test("a wide fold's horizontal crease and a window offset on the display", () => {
	// Pura X: EXPANDED is one wide screen folded across a horizontal hinge.
	const wide = decode(encodePosture(reading({
		status: 1,
		window: { left: 0, top: 100, width: 1000, height: 2000 },
		creases: [{ left: 0, top: 1100, width: 1000, height: 0 }],
	})));
	equal(wide.fold, OPEN);
	deepStrictEqual(wide.hinges[0].region, [0, 0.5, 1, 0]);
	equal(wide.hinges[0].axis, HORIZONTAL);
	equal(wide.hinges[0].angle, undefined);
});

Deno.test("trifolds pair creases with the hinges that are not folded", () => {
	const creases = [
		{ left: 1000, top: 0, width: 0, height: 1000 },
		{ left: 2000, top: 0, width: 0, height: 1000 },
	];
	const window = { left: 0, top: 0, width: 3000, height: 1000 };
	const three = decode(encodePosture(reading({
		status: 21,
		window: window,
		creases: creases,
		angles: [180, 120],
	})));
	equal(three.fold, HALF_OPEN);
	deepStrictEqual(three.hinges.map((hinge) => [hinge.fold, hinge.angle]), [
		[OPEN, 180],
		[HALF_OPEN, 120],
	]);
	deepStrictEqual(three.hinges[1].region, [0.667, 0, 0, 1]);

	// Two panels through the first hinge (Mate XT's EXPANDED), or through
	// the second hinge with the first folded away.
	const window2 = { left: 0, top: 0, width: 2000, height: 1000 };
	for (const [status, angle] of [[1, 180], [12, 170]]) {
		const two = decode(encodePosture(reading({
			status: status,
			window: window2,
			creases: [creases[0]],
			angles: [180, 170],
		})));
		equal(two.fold, OPEN);
		deepStrictEqual(two.hinges.map((hinge) => [hinge.fold, hinge.angle]), [[
			OPEN,
			angle,
		]]);
	}
});

Deno.test("folded, rigid, and windows beside the fold report no hinges", () => {
	deepStrictEqual(decode(encodePosture(reading({ status: 2 }))), {
		foldable: 2,
		fold: CLOSED,
		hinges: [],
	});
	// Flip phones answer isFoldable() with false, so it is not proof.
	deepStrictEqual(decode(encodePosture(reading({ foldable: false }))), {
		foldable: 0,
		fold: UNKNOWN,
		hinges: [],
	});
	const crease = { left: 1000, top: 0, width: 0, height: 1000 };
	for (
		const window of [
			{ left: 0, top: 0, width: 900, height: 1000 },
			{ left: 0, top: 0, width: 1000, height: 1000 },
			{ left: 1000, top: 0, width: 1000, height: 1000 },
		]
	) {
		const split = decode(
			encodePosture(reading({ status: 3, window: window, creases: [crease] })),
		);
		equal(split.fold, HALF_OPEN);
		deepStrictEqual(split.hinges, [], JSON.stringify(window));
	}
	const unplaced = decode(
		encodePosture(reading({ status: 1, window: undefined, creases: [crease] })),
	);
	deepStrictEqual(unplaced.hinges[0].region, undefined);
	equal(unplaced.hinges[0].axis, VERTICAL);
});

Deno.test("encoding matches the Rust wire format byte for byte", () => {
	const bytes = new Uint8Array(encodePosture(reading({
		status: 1,
		creases: [{ left: 1000, top: 0, width: 0, height: 1000 }],
	})));
	const expected = new DataView(new ArrayBuffer(48));
	[2, OPEN, 1, OPEN, VERTICAL, 0, 1].forEach((word, index) =>
		expected.setUint32(4 * index, word, true)
	);
	[0.5, 0, 0, 1].forEach((value, index) =>
		expected.setFloat32(28 + 4 * index, value, true)
	);
	expected.setUint32(44, 0xffffffff, true);
	deepStrictEqual(bytes, new Uint8Array(expected.buffer));
});

function plugin(): PosturePlugin {
	device.reset();
	device.status = 1;
	device.window = { left: 0, top: 0, width: 2000, height: 1000 };
	device.creases = [{ left: 1000, top: 0, width: 0, height: 1000 }];
	return new PosturePlugin({ filesDir: "/data" });
}

const call = (method: number, payload = new ArrayBuffer(0)) =>
	new NativeCall(method, payload);
const code = (expected: number) => (error: unknown) =>
	error instanceof PluginError && error.code === expected;

Deno.test("waits report fold, angle, and window changes, then stop observing on close", async () => {
	const posture = plugin();
	const open = await posture.invoke(call(0));
	equal(decode(open).fold, OPEN);

	const folding = posture.invoke(call(1, open));
	await Promise.resolve();
	device.emitStatus(3);
	equal(decode(await folding).fold, HALF_OPEN);

	const half = await posture.invoke(call(0));
	const angled = posture.invoke(call(1, half));
	await Promise.resolve();
	device.emitAngles([100]);
	equal(decode(await angled).hinges[0].angle, 100);

	const resized = posture.invoke(call(1, await posture.invoke(call(0))));
	await Promise.resolve();
	device.emitResize({ left: 0, top: 0, width: 4000, height: 1000 });
	deepStrictEqual(decode(await resized).hinges[0].region, [0.25, 0, 0, 1]);

	// A change before the wait starts is reported at once.
	equal(decode(await posture.invoke(call(1, open))).fold, HALF_OPEN);

	posture.close();
	equal(
		device.statusListeners.size + device.angleListeners.size +
			device.sizeListeners.size,
		0,
	);
	await rejects(posture.invoke(call(0)), code(CANCELLED));
});

Deno.test("one wait at a time, cancellation, and malformed payloads", async () => {
	const posture = plugin();
	const current = await posture.invoke(call(0));
	const first = call(1, current);
	const waiting = posture.invoke(first);
	await rejects(posture.invoke(call(1, current)), code(BUSY));
	first.cancel();
	await rejects(waiting, code(CANCELLED));
	// The slot is free again after cancellation.
	const again = posture.invoke(call(1, current));
	await Promise.resolve();
	device.emitStatus(2);
	equal(decode(await again).fold, CLOSED);

	for (const length of [0, 11, 13, 12 + 36 * 5]) {
		await rejects(
			posture.invoke(call(1, new ArrayBuffer(length))),
			code(INVALID_ARGUMENT),
		);
	}
	await rejects(
		posture.invoke(call(0, new ArrayBuffer(4))),
		code(INVALID_ARGUMENT),
	);
	posture.close();
});

Deno.test("devices that report no fold never subscribe", async () => {
	const posture = plugin();
	device.foldable = false;
	deepStrictEqual(decode(await posture.invoke(call(0))), {
		foldable: 0,
		fold: UNKNOWN,
		hinges: [],
	});
	equal(
		device.statusListeners.size + device.angleListeners.size +
			device.sizeListeners.size,
		0,
	);
	posture.close();
});
