// Runs first inside a played example's frame, before the app's own scripts,
// and talks to the player on the docs page (`play.js`) over `postMessage`:
//
// - `hello` as soon as it runs, so the player knows the page is a played
//   build and not, say, a missing file;
// - `ready` once the app has put its first element on the page;
// - `history` whenever the frame's own history moves, with whether it can go
//   back;
// - `error` when the app throws.
//
// The player's back button sends `back`, which goes back in this frame's
// history. Opened in a tab of its own, outside a player, it does nothing.

(() => {
	"use strict";

	const player = window.parent;
	if (player === window) return;

	/** @param {Record<string, unknown>} message */
	const post = (message) => {
		// The frame may have an opaque origin, which the player cannot be
		// addressed from, and nothing here is private.
		player.postMessage({ lk: "play", ...message }, "*");
	};

	post({ type: "hello" });

	// The entries this frame has added, oldest first, by URL: enough to know
	// whether back stays inside the example.
	const earlier = [];
	let current = location.href;
	const moved = () => {
		current = location.href;
		post({ type: "history", canGoBack: earlier.length > 0 });
	};

	const pushState = history.pushState;
	history.pushState = function (...args) {
		const before = current;
		const result = pushState.apply(this, args);
		earlier.push(before);
		moved();
		return result;
	};
	const replaceState = history.replaceState;
	history.replaceState = function (...args) {
		const result = replaceState.apply(this, args);
		moved();
		return result;
	};
	addEventListener("popstate", () => {
		if (earlier.at(-1) === location.href) earlier.pop();
		else earlier.push(current);
		moved();
	});

	addEventListener("message", (event) => {
		if (event.source !== player || event.origin !== location.origin) return;
		const data = event.data;
		if (data?.lk === "play" && data.type === "back" && earlier.length > 0) {
			history.back();
		}
	});

	addEventListener("error", (event) => {
		post({ type: "error", message: String(event.message || "an error") });
	});
	addEventListener("unhandledrejection", (event) => {
		const reason = event.reason;
		post({
			type: "error",
			message: String(reason?.message ?? reason ?? "an error"),
		});
	});

	// The app mounts by adding its first element to the body.
	const shown = () =>
		[...(document.body?.children ?? [])].some((element) =>
			!["SCRIPT", "NOSCRIPT", "STYLE", "LINK", "TEMPLATE"].includes(
				element.tagName,
			)
		);
	const watch = new MutationObserver(() => {
		if (!shown()) return;
		watch.disconnect();
		post({ type: "ready" });
	});
	watch.observe(document.documentElement, { childList: true, subtree: true });
})();
