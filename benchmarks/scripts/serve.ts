// A gzip static file server for benchmark fixtures. Payload numbers come
// from the page's own resource entries, so serving compression keeps
// transfer bytes representative of a real deployment.
import { gzipSync } from "node:zlib";

const CONTENT_TYPES: Record<string, string> = {
	".html": "text/html; charset=utf-8",
	".js": "text/javascript; charset=utf-8",
	".mjs": "text/javascript; charset=utf-8",
	".json": "application/json; charset=utf-8",
	".css": "text/css; charset=utf-8",
	".wasm": "application/wasm",
	".svg": "image/svg+xml",
	".png": "image/png",
	".ico": "image/x-icon",
	".map": "application/json; charset=utf-8",
};

export interface StaticServer {
	/** The origin serving the root directory. */
	url: string;
	close(): Promise<void>;
}

export function serveStatic(root: string): StaticServer {
	const server = Deno.serve({ hostname: "127.0.0.1", port: 0 }, (request) => {
		return handle(request, root);
	});
	return {
		url: `http://127.0.0.1:${
			server.addr.transport === "tcp" ? server.addr.port : 0
		}`,
		close: () => server.shutdown(),
	};
}

async function handle(request: Request, root: string): Promise<Response> {
	const path = new URL(request.url).pathname;
	const relative = path === "/" ? "/index.html" : path;
	const resolved = safelyJoin(root, relative);
	if (!resolved) {
		return new Response("not found", { status: 404 });
	}
	let bytes: Uint8Array;
	try {
		bytes = await Deno.readFile(resolved);
	} catch {
		return new Response("not found", { status: 404 });
	}
	const headers = new Headers({
		"content-type": CONTENT_TYPES[extensionOf(resolved)] ??
			"application/octet-stream",
		// Benchmark fixtures must never be served from cache: base and head
		// builds share URLs, and a stale entry would poison a comparison.
		"cache-control": "no-store",
	});
	const acceptsGzip = (request.headers.get("accept-encoding") ?? "").includes(
		"gzip",
	);
	const compress = acceptsGzip && bytes.length > 512;
	const body = compress ? gzipBytes(bytes) : copy(bytes);
	if (compress) {
		headers.set("content-encoding", "gzip");
	}
	headers.set("content-length", String(body.length));
	return new Response(body, { status: 200, headers });
}

/** A fresh `Uint8Array` over its own `ArrayBuffer`, as `Response` requires. */
function copy(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
	const owned = new Uint8Array(bytes.byteLength);
	owned.set(bytes);
	return owned;
}

function gzipBytes(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
	return copy(gzipSync(bytes, { level: 6 }));
}

function extensionOf(path: string): string {
	const name = path.replaceAll("\\", "/").split("/").pop() ?? "";
	const dot = name.lastIndexOf(".");
	return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

/** Resolves `relative` under `root`, rejecting traversal outside it. */
function safelyJoin(root: string, relative: string): string | null {
	const rootPath = Deno.realPathSync(root);
	const decoded = decodeURIComponent(relative);
	const target = `${rootPath}/${decoded.replaceAll(/^\/+|\.\./g, "")}`;
	if (!target.startsWith(`${rootPath}/`)) return null;
	return target;
}
