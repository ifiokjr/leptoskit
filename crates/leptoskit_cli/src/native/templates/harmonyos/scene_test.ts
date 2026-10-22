import { deepStrictEqual, equal, throws } from "node:assert/strict";
import { color, Scene } from "./scene.ts";

Deno.test("native mutations preserve layout, child order, and controlled text", () => {
	const scene = new Scene();
	const initial = scene.apply({
		version: 1,
		root: 0,
		mutations: [
			{ Create: { id: 0, kind: "view", props: {} } },
			{ Create: { id: 1, kind: "text_input", props: { value: "before" } } },
			{ Create: { id: 2, kind: "button", props: { text: "Go" } } },
			{ InsertChild: { parent: 0, child: 1, index: 0 } },
			{ InsertChild: { parent: 0, child: 2, index: 1 } },
			{ UpdateLayout: { id: 1, x: 12, y: 14, width: 300, height: 44 } },
		],
	});
	equal(initial?.children[0].props.value, "before");
	equal(initial?.children[0].frame.width, 300);
	const next = scene.apply({
		version: 1,
		root: 0,
		mutations: [
			{ UpdateProps: { id: 1, props: { value: "after" } } },
			{ InsertChild: { parent: 0, child: 2, index: 0 } },
		],
	});
	deepStrictEqual(next?.children.map((child) => child.id), [2, 1]);
	equal(next?.children[1].props.value, "after");
	equal(initial?.children[0].props.value, "before");
	equal(next?.contentWidth(), 312);
	equal(next?.contentHeight(), 58);
	const removed = scene.apply({
		version: 1,
		root: 0,
		mutations: [
			{ RemoveChild: { parent: 0, child: 1 } },
			{ Delete: { id: 1 } },
		],
	});
	deepStrictEqual(removed?.children.map((child) => child.id), [2]);
	equal(scene.apply({ version: 1, root: null, mutations: [] }), undefined);
});

Deno.test("unsupported widgets, protocol versions, and remote images fail visibly", () => {
	throws(
		() => new Scene().apply({ version: 2, root: null, mutations: [] }),
		/protocol/,
	);
	throws(() =>
		new Scene().apply({
			version: 1,
			root: 0,
			mutations: [
				{ Create: { id: 0, kind: "unregistered", props: {} } },
			],
		}), /Unsupported HarmonyOS widget/);
	throws(() =>
		new Scene().apply({
			version: 1,
			root: 0,
			mutations: [
				{
					Create: {
						id: 0,
						kind: "image",
						props: { source: "https://example.com/image.png" },
					},
				},
			],
		}), /file URL/);
});

Deno.test("packed RGBA becomes unsigned ArkUI ARGB, including transparent colors", () => {
	equal(color(0x12345678, 0), 0x78123456);
	equal(color(0, 0xffffffff), 0);
	equal(color(undefined, 0xff112233), 0xff112233);
});
