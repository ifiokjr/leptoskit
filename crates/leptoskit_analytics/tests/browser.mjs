// Deterministic SDK tests with browser APIs mocked; no server or network.
import { install } from "../src/browser.mjs";

const check = (condition, message) => {
	if (!condition) throw new Error(message);
};
const listeners = new Map();
globalThis.addEventListener = (name, fn) => listeners.set(name, fn);
globalThis.removeEventListener = (name) => listeners.delete(name);
globalThis.document = {
	readyState: "loading",
	visibilityState: "visible",
	addEventListener,
	removeEventListener,
};
Object.defineProperty(globalThis, "navigator", {
	value: { globalPrivacyControl: false, doNotTrack: "0" },
	configurable: true,
});
globalThis.location = { origin: "https://app.example.com" };
const requests = [];
globalThis.fetch = async (request, options) => {
	requests.push({ request, options });
	return new Response("ok");
};

const analytics = install({ route: "/notes/{id}" });
await analytics.tracedFetch("/api/notes?token=private-query");
const first = requests[0].request;
const context = first.headers.get("traceparent");
check(
	first.redirect === "error",
	"instrumented redirects cannot leak trace context",
);
check(/^00-[a-f0-9]{32}-[a-f0-9]{16}-00$/.test(context), "valid traceparent");
await analytics.tracedFetch("https://another.example.com/external");
check(
	!requests[1].request.headers.has("traceparent"),
	"no cross-origin trace context",
);
listeners.get("error")({
	message: "private-error-message",
	filename: "private-source",
});
await analytics.flush();
const batch = requests[2];
check(batch.options.credentials === "omit", "ingestion omits credentials");
check(batch.options.mode === "same-origin", "same-origin ingestion");
const payload = JSON.parse(batch.options.body);
check(
	payload.events.length === 3,
	"pageview, selected request, and error recorded",
);
check(payload.events[1].traceparent === context, "request context correlated");
check(
	payload.events.every((e) => e.route === "/notes/{id}"),
	"only route templates",
);
check(
	!/private-/.test(batch.options.body),
	"no URLs or error messages retained",
);
for (let i = 0; i < 50; i++) analytics.pageView("/notes/{id}");
check(analytics.dropped === 18, "bounded queue with visible loss");
await analytics.flush();
analytics.pageView("/path?email=private@example.com");
await analytics.flush();
check(requests.length === 4, "invalid routes are not sent");
analytics.stop();
check(listeners.size === 0, "listeners removed");

for (const preference of ["globalPrivacyControl", "doNotTrack"]) {
	navigator[preference] = preference === "globalPrivacyControl" ? true : "1";
	const before = requests.length;
	const optedOut = install({ route: "/notes/{id}" });
	optedOut.pageView("/notes/{id}");
	await optedOut.flush();
	check(requests.length === before, "privacy preference prevents collection");
	optedOut.stop();
	navigator[preference] = preference === "globalPrivacyControl" ? false : "0";
}
const disabled = install({ route: "/notes/{id}", enabled: false });
await disabled.flush();
disabled.stop();
const originalEntries = performance.getEntriesByType;
performance.getEntriesByType = () => [{ duration: 125 }];
const navigation = install({ route: "/notes/{id}" });
listeners.get("load")();
await new Promise((resolve) => setTimeout(resolve, 1));
await navigation.flush();
const navigationEvents = JSON.parse(requests.at(-1).options.body).events;
check(
	navigationEvents.filter((e) => e.kind === "navigation").length === 1,
	"navigation measured after load",
);
navigation.stop();
performance.getEntriesByType = originalEntries;
console.log(
	"Browser analytics SDK: correlation, privacy, bounds and lifecycle passed.",
);
