// Explicitly install after your application's consent decision. This module
// never reads a cookie, URL path/query, DOM text, exception message or user ID.
const endpoint = "/_leptoskit/analytics/browser";
const validRoute = (route) =>
	typeof route === "string" && /^[A-Za-z0-9/:_.*{}-]{1,128}$/.test(route);

export function install({ route, enabled = true } = {}) {
	let active = enabled && navigator.globalPrivacyControl !== true &&
		navigator.doNotTrack !== "1";
	let current = validRoute(route) ? route : null;
	let pending = [];
	let dropped = 0;
	const emit = (
		kind,
		duration_ms = 0,
		traceparent = null,
		operation = current,
	) => {
		if (!active || !operation) return;
		if (pending.length >= 32) {
			dropped++;
			return;
		}
		pending.push({ kind, route: operation, duration_ms, traceparent });
	};
	const flush = async () => {
		if (!active || pending.length === 0) return;
		const events = pending;
		pending = [];
		try {
			const response = await fetch(endpoint, {
				method: "POST",
				mode: "same-origin",
				credentials: "omit",
				keepalive: true,
				headers: { "content-type": "application/json" },
				body: JSON.stringify({ events }),
			});
			if (!response.ok) dropped += events.length;
		} catch {
			dropped += events.length;
		}
	};
	const error = () => emit("error");
	const initialRoute = current;
	let navigationRecorded = false;
	const navigation = () => {
		if (navigationRecorded) return;
		const timing = performance.getEntriesByType("navigation")[0];
		if (timing && timing.duration > 0 && Number.isFinite(timing.duration)) {
			navigationRecorded = true;
			emit(
				"navigation",
				Math.min(86400000, timing.duration),
				null,
				initialRoute,
			);
		}
	};
	// loadEventEnd is populated after load listeners return.
	const loaded = () => {
		loadTimer = setTimeout(navigation, 0);
	};
	const visible = () => {
		if (document.visibilityState === "hidden") void flush();
	};
	let interval;
	let loadTimer;
	if (active) {
		emit("page_view");
		addEventListener("error", error);
		addEventListener("unhandledrejection", error);
		addEventListener("load", loaded, { once: true });
		document.addEventListener("visibilitychange", visible);
		if (document.readyState === "complete") {
			loaded();
		}
		interval = setInterval(flush, 10000);
	}
	return {
		// Call from the app router with its route template, e.g. /notes/{id}.
		pageView(next) {
			current = validRoute(next) ? next : null;
			emit("page_view");
		},
		flush,
		get dropped() {
			return dropped;
		},
		// Opt-in request instrumentation; does not patch the global fetch API.
		// Context stays on this origin. Server-side contexts are correlation IDs,
		// not permission to change infrastructure or trust browser reports.
		async tracedFetch(input, init) {
			const request = new Request(
				input instanceof Request ? input : new URL(input, location.origin),
				init,
			);
			if (!active || new URL(request.url).origin !== location.origin) {
				return fetch(request);
			}
			const bytes = crypto.getRandomValues(new Uint8Array(24));
			if (bytes.slice(0, 16).every((b) => b === 0)) bytes[0] = 1;
			if (bytes.slice(16).every((b) => b === 0)) bytes[16] = 1;
			const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join(
				"",
			);
			const context = `00-${hex.slice(0, 32)}-${hex.slice(32)}-00`;
			request.headers.set("traceparent", context);
			const started = performance.now();
			// Capture the application template before navigation changes it.
			const operation = current;
			const record = (kind) => {
				emit(
					kind,
					Math.min(86400000, Math.max(0, performance.now() - started)),
					context,
					operation,
				);
			};
			try {
				// Reject redirects so a same-origin endpoint cannot forward our
				// correlation header to another origin.
				const response = await fetch(
					new Request(request, { redirect: "error" }),
				);
				record(response.ok ? "request" : "request_failed");
				return response;
			} catch (failure) {
				record("request_failed");
				throw failure;
			}
		},
		stop() {
			active = false;
			pending = [];
			clearInterval(interval);
			clearTimeout(loadTimer);
			removeEventListener("error", error);
			removeEventListener("unhandledrejection", error);
			removeEventListener("load", loaded);
			document.removeEventListener("visibilitychange", visible);
		},
	};
}
