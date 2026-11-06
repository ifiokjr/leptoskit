import { deepStrictEqual, equal, throws } from "node:assert/strict";
import { borderEdges, color, isItalic, Scene, textContent } from "./scene.ts";

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

Deno.test("Rive snapshots retain identity through updates and retire deleted canvases", () => {
	const scene = new Scene();
	const initial = scene.apply({
		version: 1,
		root: 1,
		mutations: [
			{
				Create: {
					id: 1,
					kind: "rive",
					props: { accessibility_label: "Progress" },
				},
			},
			{ UpdateLayout: { id: 1, x: 0, y: 0, width: 128, height: 64 } },
		],
	});
	const retained = scene.node(1);
	const updated = scene.apply({
		version: 1,
		root: 1,
		mutations: [
			{ UpdateProps: { id: 1, props: { aria: { aria_hidden: true } } } },
		],
	});
	equal(scene.node(1), retained);
	equal(initial?.props.accessibility_label, "Progress");
	equal(updated?.props.aria?.aria_hidden, true);
	equal(updated?.frame.width, 128);
	equal(
		scene.apply({ version: 1, root: null, mutations: [{ Delete: { id: 1 } }] }),
		undefined,
	);
	equal(scene.nodes.has(1), false);
	throws(() => scene.node(1), /Unknown render node/);
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

Deno.test("text fields name their content from the first autocomplete token password managers read", () => {
	equal(textContent("username webauthn"), "username");
	equal(textContent("section-login Email"), "email");
	equal(textContent("current-password"), "current-password");
	equal(textContent("new-password"), "new-password");
	equal(textContent("one-time-code"), undefined);
	equal(textContent(null), undefined);
	equal(textContent(undefined), undefined);
});

Deno.test("border edges prefer per-edge widths and keep missing edges undrawn", () => {
	deepStrictEqual(borderEdges({}), {
		top: 0,
		right: 0,
		bottom: 0,
		left: 0,
	});
	deepStrictEqual(borderEdges({ border_width: 2 }), {
		top: 2,
		right: 2,
		bottom: 2,
		left: 2,
	});
	deepStrictEqual(
		borderEdges({
			border_width: 4,
			border_widths: { top: 1, right: 0, bottom: 0, left: 2 },
		}),
		{ top: 1, right: 0, bottom: 0, left: 2 },
	);
	equal(isItalic(true), true);
	equal(isItalic(false), false);
	equal(isItalic(null), false);
	equal(isItalic(undefined), false);
});

/**
 * Yoga frames from `grid_parity_probe` for the same Tailwind trees the web
 * HTML probe measures. The ArkUI host places each widget with absolute
 * `.position` / `.width` / `.height` from these frames and must not reflow.
 */
const GRID_PARITY_FRAMES = {
	"grid-cols-3-gap": {
		root: { x: 0, y: 0, width: 400, height: 200 },
		children: [
			{ id: "cell-0", x: 16, y: 16, width: 112, height: 76 },
			{ id: "cell-1", x: 144, y: 16, width: 112, height: 76 },
			{ id: "cell-2", x: 272, y: 16, width: 112, height: 76 },
			{ id: "cell-3", x: 16, y: 108, width: 112, height: 76 },
			{ id: "cell-4", x: 144, y: 108, width: 112, height: 76 },
			{ id: "cell-5", x: 272, y: 108, width: 112, height: 76 },
		],
	},
	"flex-row-gap-grow": {
		root: { x: 0, y: 0, width: 400, height: 80 },
		children: [
			{ id: "fixed", x: 8, y: 8, width: 64, height: 64 },
			{ id: "grow", x: 84, y: 8, width: 216, height: 64 },
			{ id: "tail", x: 312, y: 8, width: 80, height: 64 },
		],
	},
	"grid-align-justify-items": {
		root: { x: 0, y: 0, width: 300, height: 200 },
		children: [
			{ id: "dot-0", x: 51, y: 26, width: 40, height: 40 },
			{ id: "dot-1", x: 209, y: 26, width: 40, height: 40 },
			{ id: "dot-2", x: 51, y: 134, width: 40, height: 40 },
			{ id: "dot-3", x: 209, y: 134, width: 40, height: 40 },
		],
	},
	"nested-grid-in-flex": {
		root: { x: 0, y: 0, width: 480, height: 240 },
		children: [
			{ id: "side", x: 16, y: 16, width: 160, height: 208 },
			{
				id: "nest",
				x: 192,
				y: 16,
				width: 272,
				height: 208,
				children: [
					{ id: "nest-0", x: 0, y: 0, width: 132, height: 100 },
					{ id: "nest-1", x: 140, y: 0, width: 132, height: 100 },
					{ id: "nest-2", x: 0, y: 108, width: 132, height: 100 },
					{ id: "nest-3", x: 140, y: 108, width: 132, height: 100 },
				],
			},
		],
	},
} as const;

Deno.test("Scene keeps Yoga grid_parity_probe frames for ArkUI absolute placement", () => {
	for (const [name, fixture] of Object.entries(GRID_PARITY_FRAMES)) {
		const scene = new Scene();
		const mutations: Array<Record<string, unknown>> = [
			{
				Create: {
					id: 0,
					kind: "view",
					props: { background: 0xf8fafcff },
				},
			},
			{
				UpdateLayout: {
					id: 0,
					...fixture.root,
				},
			},
		];
		let nextId = 1;
		const allocate = (
			nodes: ReadonlyArray<{
				id: string;
				x: number;
				y: number;
				width: number;
				height: number;
				children?: ReadonlyArray<{
					id: string;
					x: number;
					y: number;
					width: number;
					height: number;
				}>;
			}>,
			parent: number,
		) => {
			nodes.forEach((node, index) => {
				const id = nextId++;
				mutations.push({
					Create: {
						id,
						kind: "view",
						props: {
							accessibility_label: node.id,
							background: 0xcbd5e1ff,
							border_widths: { top: 1, right: 0, bottom: 0, left: 2 },
							border_color: 0x64748bff,
							italic: name.includes("align") ? true : null,
						},
					},
				});
				mutations.push({
					UpdateLayout: {
						id,
						x: node.x,
						y: node.y,
						width: node.width,
						height: node.height,
					},
				});
				mutations.push({ InsertChild: { parent, child: id, index } });
				if (node.children) {
					allocate(node.children, id);
				}
			});
		};
		allocate(fixture.children, 0);
		const root = scene.apply({ version: 1, root: 0, mutations });
		equal(root?.frame.width, fixture.root.width, name);
		equal(root?.frame.height, fixture.root.height, name);
		equal(root?.children.length, fixture.children.length, name);
		fixture.children.forEach((expected, index) => {
			const child = root?.children[index];
			equal(child?.props.accessibility_label, expected.id, name);
			deepStrictEqual(
				{
					x: child?.frame.x,
					y: child?.frame.y,
					width: child?.frame.width,
					height: child?.frame.height,
				},
				{
					x: expected.x,
					y: expected.y,
					width: expected.width,
					height: expected.height,
				},
				`${name}/${expected.id}`,
			);
			deepStrictEqual(
				borderEdges(child?.props ?? {}),
				{ top: 1, right: 0, bottom: 0, left: 2 },
				`${name}/${expected.id} edges`,
			);
			if ("children" in expected && expected.children) {
				equal(child?.children.length, expected.children.length, name);
				expected.children.forEach((nested, nestedIndex) => {
					const grandchild = child?.children[nestedIndex];
					deepStrictEqual(
						{
							x: grandchild?.frame.x,
							y: grandchild?.frame.y,
							width: grandchild?.frame.width,
							height: grandchild?.frame.height,
						},
						{
							x: nested.x,
							y: nested.y,
							width: nested.width,
							height: nested.height,
						},
						`${name}/${nested.id}`,
					);
				});
			}
		});
	}
});
