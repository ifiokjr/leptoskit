// A minimal Chrome DevTools Protocol client: launch a pinned-flag headless
// Chromium, open one page, evaluate expressions, and count network bytes.
// The harness speaks CDP directly so the benchmark toolchain stays inside
// this repository — no browser-driver downloads, no Node services.
//
// The flag set matches what js-framework-benchmark-style harnesses use:
// headless, background throttling disabled, scrollbars hidden. Every run
// uses the same flags, which is what makes the numbers comparable.

export interface Page {
	/** Evaluates an expression in the page, awaiting any returned promise. */
	evaluate<T>(expression: string): Promise<T>;
	/** Navigates to a URL and resolves once the load event fired. */
	navigate(url: string): Promise<void>;
	/** Network bytes received since the last navigation. */
	networkBytes(): number;
	/** Console errors and uncaught exceptions since the last navigation. */
	pageErrors(): string[];
	close(): Promise<void>;
}

export interface Browser {
	version: string;
	newPage(): Promise<Page>;
	close(): Promise<void>;
}

interface CdpMessage {
	id?: number;
	method?: string;
	params?: unknown;
	result?: unknown;
	error?: { message: string };
}

const BROWSER_CANDIDATES = [
	"chromium",
	"chromium-browser",
	"google-chrome",
	"google-chrome-stable",
];

function findBrowserBinary(): string {
	const override = Deno.env.get("LEPTOSKIT_BENCH_BROWSER");
	if (override) return override;
	for (const candidate of BROWSER_CANDIDATES) {
		const which = new Deno.Command("which", { args: [candidate] }).outputSync();
		if (which.success) {
			return new TextDecoder().decode(which.stdout).trim();
		}
	}
	const mac = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
	if (Deno.statSync(mac).isFile) return mac;
	throw new Error(
		"no Chromium found; set LEPTOSKIT_BENCH_BROWSER to a Chromium binary",
	);
}

class PageConnection {
	readonly socket: WebSocket;
	nextId = 1;
	pending = new Map<
		number,
		{ resolve: (value: unknown) => void; reject: (error: Error) => void }
	>();
	eventHandlers = new Map<string, (params: unknown) => void>();

	constructor(readonly url: string) {
		this.socket = new WebSocket(url);
		this.socket.onmessage = (event) =>
			this.handle(JSON.parse(String(event.data)));
	}

	async open(): Promise<void> {
		if (this.socket.readyState === WebSocket.OPEN) return;
		await new Promise<void>((resolve, reject) => {
			this.socket.onopen = () => resolve();
			this.socket.onerror = () =>
				reject(new Error(`could not connect to ${this.url}`));
		});
		await this.send("Runtime.enable");
		await this.send("Page.enable");
		await this.send("Network.enable");
	}

	on(method: string, handler: (params: unknown) => void): void {
		this.eventHandlers.set(method, handler);
	}

	private handle(message: CdpMessage): void {
		if (message.id !== undefined) {
			const entry = this.pending.get(message.id);
			if (!entry) return;
			this.pending.delete(message.id);
			if (message.error) entry.reject(new Error(message.error.message));
			else entry.resolve(message.result);
			return;
		}
		if (message.method) {
			this.eventHandlers.get(message.method)?.(message.params);
		}
	}

	send(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
		const id = this.nextId++;
		const text = JSON.stringify({ id, method, params });
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
			this.socket.send(text);
			setTimeout(() => {
				if (this.pending.delete(id)) {
					reject(new Error(`CDP command ${method} timed out`));
				}
			}, 60_000);
		});
	}

	close(): void {
		for (const entry of this.pending.values()) {
			entry.reject(new Error("connection closed"));
		}
		this.pending.clear();
		this.socket.close();
	}
}

class CdpPage implements Page {
	#bytes = 0;
	#errors: string[] = [];
	#loaded: (() => void) | null = null;

	constructor(
		private readonly connection: PageConnection,
		private readonly targetId: string,
		private readonly closeTarget: () => Promise<void>,
	) {
		connection.on("Network.dataReceived", (params) => {
			const { dataLength } = params as { dataLength: number };
			this.#bytes += dataLength;
		});
		connection.on("Page.loadEventFired", () => this.#loaded?.());
		connection.on("Runtime.exceptionThrown", (params) => {
			const { exceptionDetails } = params as {
				exceptionDetails: { text: string; exception?: { description: string } };
			};
			this.#errors.push(
				exceptionDetails.exception?.description ?? exceptionDetails.text,
			);
		});
		connection.on("Runtime.consoleAPICalled", (params) => {
			const { type, args } = params as {
				type: string;
				args: { value?: unknown; description?: string }[];
			};
			if (type === "error") {
				this.#errors.push(
					args.map((arg) => arg.description ?? String(arg.value)).join(" "),
				);
			}
		});
	}

	async evaluate<T>(expression: string): Promise<T> {
		const result = await this.connection.send("Runtime.evaluate", {
			expression,
			returnByValue: true,
			awaitPromise: true,
		}) as {
			result?: { value?: T };
			exceptionDetails?: { exception?: { description?: string }; text: string };
		};
		if (result.exceptionDetails) {
			const detail = result.exceptionDetails.exception?.description ??
				result.exceptionDetails.text;
			throw new Error(`page evaluation failed: ${detail}`);
		}
		return result.result?.value as T;
	}

	async navigate(url: string): Promise<void> {
		this.#bytes = 0;
		this.#errors = [];
		const loaded = new Promise<void>((resolve) => {
			this.#loaded = resolve;
		});
		await this.connection.send("Page.navigate", { url });
		await Promise.race([
			loaded,
			new Promise((_resolve, reject) =>
				setTimeout(
					() => reject(new Error(`navigation to ${url} timed out`)),
					60_000,
				)
			),
		]);
	}

	networkBytes(): number {
		return this.#bytes;
	}

	pageErrors(): string[] {
		return [...this.#errors];
	}

	async close(): Promise<void> {
		this.connection.close();
		await this.closeTarget();
		void this.targetId;
	}
}

// How long a launch may take to expose its DevTools endpoint. Startup is not
// measured, so the budget only has to cover the slowest legitimate start: the
// first launch on a fresh hosted runner reads the browser binary and its
// resources from a cold disk, which measured 8–20 s (every later launch on the
// same runner takes ~0.2 s), and a 30 s budget failed intermittently.
const LAUNCH_TIMEOUT_MS = 120_000;

// The most recent browser output kept for error reports. Older output is
// dropped: the browser logs for as long as it runs, and only the lines around
// a failure explain it.
const STDERR_TAIL_BYTES = 64 * 1024;

/**
 * Drains the browser's stderr for the whole life of the process.
 *
 * The browser keeps writing diagnostics after it is ready (for example a dbus
 * error each time a service lookup fails), and a pipe nobody reads fills up
 * and blocks the browser mid-run. Draining continuously keeps it writing,
 * while the tail stays available for any later error.
 */
class StderrTail {
	#text = "";
	#endpoint?: string;
	/** Resolves with the DevTools URL, or `undefined` if the stream ends first. */
	readonly endpoint: Promise<string | undefined>;
	/** Resolves once the browser closes its stderr. */
	readonly closed: Promise<void>;

	constructor(stream: ReadableStream<Uint8Array>) {
		let found: (endpoint: string | undefined) => void = () => {};
		this.endpoint = new Promise((resolve) => {
			found = resolve;
		});
		this.closed = (async () => {
			const decoder = new TextDecoder();
			for await (const chunk of stream) {
				this.#text = (this.#text + decoder.decode(chunk, { stream: true }))
					.slice(-STDERR_TAIL_BYTES);
				if (this.#endpoint === undefined) {
					const match = this.#text.match(/DevTools listening on (ws:\/\/\S+)/);
					if (match) {
						this.#endpoint = match[1];
						found(this.#endpoint);
					}
				}
			}
			found(undefined);
		})();
	}

	text(): string {
		return this.#text;
	}
}

export async function launchBrowser(): Promise<Browser> {
	const binary = findBrowserBinary();
	const userDataDir = await Deno.makeTempDir({ prefix: "leptoskit-bench-" });
	const process = new Deno.Command(binary, {
		args: [
			"--headless=new",
			// The fixtures are this repository's own pages, and hosted CI runners
			// (Ubuntu 24.04 restricts unprivileged user namespaces) cannot start
			// Chromium's sandbox; without this the browser exits at launch.
			"--no-sandbox",
			"--disable-dev-shm-usage",
			"--remote-debugging-port=0",
			`--user-data-dir=${userDataDir}`,
			"--no-first-run",
			"--no-default-browser-check",
			"--disable-background-timer-throttling",
			"--disable-backgrounding-occluded-windows",
			"--disable-renderer-backgrounding",
			"--disable-features=CalculateNativeWinOcclusion",
			"--hide-scrollbars",
			// The scenario durations end on the second animation frame; with
			// vsync pacing that alone is a ~33 ms floor that drowns every fast
			// scenario. Unthrottled frames keep the settle cost in the noise.
			"--disable-gpu-vsync",
			"--disable-frame-rate-limit",
			"--js-flags=--expose-gc",
			"about:blank",
		],
		stderr: "piped",
		stdout: "null",
	}).spawn();

	const stderr = new StderrTail(process.stderr);
	const launched = performance.now();
	const elapsed = () =>
		`${((performance.now() - launched) / 1000).toFixed(1)}s`;
	const failure = async (reason: string): Promise<Error> => {
		try {
			process.kill("SIGKILL");
		} catch {
			// already exited
		}
		const status = await process.status;
		await stderr.closed;
		await Deno.remove(userDataDir, { recursive: true }).catch(() => {});
		return new Error(
			`browser ${reason} after ${elapsed()} (exit code ${status.code}${
				status.signal ? `, signal ${status.signal}` : ""
			}); ${binary} stderr:\n${stderr.text()}`,
		);
	};

	let timer: ReturnType<typeof setTimeout> | undefined;
	const endpoint = await Promise.race([
		stderr.endpoint,
		new Promise<null>((resolve) => {
			timer = setTimeout(() => resolve(null), LAUNCH_TIMEOUT_MS);
		}),
	]);
	clearTimeout(timer);
	if (endpoint === undefined) {
		throw await failure("exited before exposing a DevTools endpoint");
	}
	if (endpoint === null) {
		throw await failure("did not expose a DevTools endpoint");
	}

	const port = new URL(endpoint).port;
	const httpBase = `http://127.0.0.1:${port}`;

	const createPage = async () => {
		const created = await fetch(`${httpBase}/json/new?about:blank`, {
			method: "PUT",
		})
			.then((response) =>
				response.json() as Promise<{ id: string; webSocketDebuggerUrl: string }>
			);
		const connection = new PageConnection(created.webSocketDebuggerUrl);
		await connection.open();
		const closeTarget = async () => {
			await fetch(`${httpBase}/json/close/${created.id}`).catch(() => {});
		};
		return new CdpPage(connection, created.id, closeTarget);
	};

	return {
		version: "",
		newPage: createPage,
		close: async () => {
			process.kill();
			await process.status;
			await stderr.closed;
			await Deno.remove(userDataDir, { recursive: true }).catch(() => {});
		},
	};
}
