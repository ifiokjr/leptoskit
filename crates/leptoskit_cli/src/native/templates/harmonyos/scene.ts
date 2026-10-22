import type { Frame, Transaction, WidgetProps } from "libleptoskit.so";
/** Yoga frames are logical vp; ArkUI must not independently reflow the tree. */
export class Widget {
	id: number;
	kind: string;
	props: WidgetProps;
	frame: Frame;
	children: Widget[] = [];

	constructor(id: number, kind: string, props: WidgetProps) {
		this.id = id;
		this.kind = kind;
		this.props = props;
		this.frame = { id: id, x: 0, y: 0, width: 0, height: 0 };
	}

	contentWidth(): number {
		let width = this.frame.width;
		this.children.forEach((child: Widget) => {
			width = Math.max(width, child.frame.x + child.frame.width);
		});
		return width;
	}

	contentHeight(): number {
		let height = this.frame.height;
		this.children.forEach((child: Widget) => {
			height = Math.max(height, child.frame.y + child.frame.height);
		});
		return height;
	}
}

/** Mutations keep widget identities. Snapshots publish one complete UI update. */
export class Scene {
	nodes: Map<number, Widget> = new Map();

	node(id: number): Widget {
		const node = this.nodes.get(id);
		if (node === undefined) {
			throw new Error(`Unknown render node ${id}`);
		}
		return node;
	}

	snapshot(node: Widget): Widget {
		const copy = new Widget(node.id, node.kind, node.props);
		copy.frame = node.frame;
		copy.children = node.children.map((child: Widget) => this.snapshot(child));
		return copy;
	}

	apply(transaction: Transaction): Widget | undefined {
		if (transaction.version !== 1) {
			throw new Error("Unsupported native render protocol");
		}
		transaction.mutations.forEach((change) => {
			if (change.Create !== undefined) {
				const created = change.Create;
				if (
					![
						"view",
						"text",
						"button",
						"text_input",
						"password_input",
						"scroll_view",
						"image",
					].includes(created.kind)
				) {
					throw new Error(`Unsupported HarmonyOS widget: ${created.kind}`);
				}
				if (this.nodes.has(created.id)) {
					throw new Error(`Duplicate render node ${created.id}`);
				}
				this.nodes.set(
					created.id,
					new Widget(created.id, created.kind, created.props),
				);
			} else if (change.UpdateProps !== undefined) {
				this.node(change.UpdateProps.id).props = change.UpdateProps.props;
			} else if (change.UpdateLayout !== undefined) {
				this.node(change.UpdateLayout.id).frame = change.UpdateLayout;
			} else if (change.InsertChild !== undefined) {
				const edge = change.InsertChild;
				const parent = this.node(edge.parent);
				const child = this.node(edge.child);
				parent.children = parent.children.filter((item: Widget) =>
					item.id !== child.id
				);
				parent.children.splice(edge.index, 0, child);
			} else if (change.RemoveChild !== undefined) {
				const edge = change.RemoveChild;
				const parent = this.node(edge.parent);
				parent.children = parent.children.filter((item: Widget) =>
					item.id !== edge.child
				);
			} else if (change.Delete !== undefined) {
				this.nodes.delete(change.Delete.id);
			} else {
				throw new Error("Unknown native render mutation");
			}
		});
		this.nodes.forEach((node: Widget) => {
			if (node.props.source && !node.props.source.startsWith("file://")) {
				throw new Error(
					"HarmonyOS images require an application-private file URL",
				);
			}
		});
		return transaction.root === null
			? undefined
			: this.snapshot(this.node(transaction.root));
	}
}

/** Convert the renderer's packed RGBA to ArkUI's ARGB value. */
export function color(
	rgba: number | null | undefined,
	fallback: number,
): number {
	return rgba === undefined || rgba === null
		? fallback
		: ((rgba >>> 8) | ((rgba & 255) << 24)) >>> 0;
}
