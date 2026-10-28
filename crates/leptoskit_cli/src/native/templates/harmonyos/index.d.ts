/** Rust's v1 render protocol. Optional values may be serialized as null. */
export interface WidgetProps {
	text?: string | null;
	source?: string | null;
	placeholder?: string | null;
	value?: string | null;
	color?: number | null;
	background?: number | null;
	enabled?: boolean | null;
	accessibility_label?: string | null;
	corner_radius?: number | null;
	border_width?: number | null;
	border_color?: number | null;
	opacity?: number | null;
	hidden?: boolean | null;
	clip?: boolean | null;
	font_size?: number | null;
	font_weight?: string | null;
	text_align?: string | null;
}
export interface Frame {
	id: number;
	x: number;
	y: number;
	width: number;
	height: number;
}
export interface Created {
	id: number;
	kind: string;
	props: WidgetProps;
}
export interface Updated {
	id: number;
	props: WidgetProps;
}
export interface Edge {
	parent: number;
	child: number;
	index: number;
}
export interface Detached {
	parent: number;
	child: number;
}
export interface Removed {
	id: number;
}
/** Externally tagged Rust enum: exactly one member is present per mutation. */
export interface Mutation {
	Create?: Created;
	UpdateProps?: Updated;
	UpdateLayout?: Frame;
	InsertChild?: Edge;
	RemoveChild?: Detached;
	Delete?: Removed;
}
export interface Transaction {
	version: number;
	root: number | null;
	mutations: Mutation[];
}
/** A guest call to a registered ArkTS plugin. */
export interface NativeRequest {
	id: number;
	plugin: number;
	method: number;
	payload: ArrayBuffer;
}
export interface RenderBatch {
	pending: boolean;
	render: Transaction;
	/** New calls for ArkTS plugins, after `cancelled` is applied. */
	requests: NativeRequest[];
	/** IDs of earlier calls the guest no longer awaits. */
	cancelled: number[];
}
/** UI-thread guest; close is idempotent. Other calls after close throw. */
export class Runtime {
	constructor(wasm: ArrayBuffer, dataDir: string, cacheDir: string);
	/** Registers an ArkTS plugin and grants its methods; returns its ID. */
	register(name: string, methods: number[]): number;
	/** Settles a call with 0 and reply bytes, or a bridge error code. */
	complete(id: number, code: number, payload: ArrayBuffer): RenderBatch;
	start(width: number, height: number): RenderBatch;
	resize(width: number, height: number, dark: boolean): RenderBatch;
	event(id: number, kind: number, text: string): RenderBatch;
	pump(): RenderBatch;
	close(): void;
}
