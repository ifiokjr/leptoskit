// leptoskit_docs: progressive enhancement for the static pages.
//
// Every page reads in full without this script. It adds the theme switch, the
// chapter drawer on small screens, search, copy buttons, tabs, the current
// section in "On this page", and live reload under `Site::serve`.

(() => {
	"use strict";

	const html = document.documentElement;
	const root = html.dataset.root || "";

	/** @param {string} key @returns {string | null} */
	const load = (key) => {
		try {
			return localStorage.getItem(key);
		} catch {
			return null;
		}
	};

	/** @param {string} key @param {string} value */
	const save = (key, value) => {
		try {
			localStorage.setItem(key, value);
		} catch {
			// Storage can be disabled; the choice just does not persist.
		}
	};

	// Theme -------------------------------------------------------------------

	const themeButton = document.querySelector("[data-lk-theme]");
	if (themeButton) {
		themeButton.hidden = false;
		themeButton.addEventListener("click", () => {
			const dark = html.dataset.theme === "dark" ||
				(!html.dataset.theme &&
					matchMedia("(prefers-color-scheme: dark)").matches);
			html.dataset.theme = dark ? "light" : "dark";
			save("lk-theme", html.dataset.theme);
		});
	}

	// Chapter drawer ----------------------------------------------------------

	const menu = document.querySelector("[data-lk-menu]");
	/** @param {boolean} open */
	const setDrawer = (open) => {
		document.body.toggleAttribute("data-nav-open", open);
		menu?.setAttribute("aria-expanded", String(open));
	};
	menu?.addEventListener(
		"click",
		() => setDrawer(!document.body.hasAttribute("data-nav-open")),
	);
	document.querySelector(".lk-main")?.addEventListener(
		"click",
		() => setDrawer(false),
	);
	document.addEventListener("keydown", (event) => {
		if (event.key === "Escape") setDrawer(false);
	});
	document.querySelector('.lk-sidebar [aria-current="page"]')?.scrollIntoView({
		block: "center",
	});

	// Copy buttons ------------------------------------------------------------

	for (const figure of document.querySelectorAll(".lk-code")) {
		const code = figure.querySelector("pre");
		if (!code || !navigator.clipboard) continue;

		const button = document.createElement("button");
		button.type = "button";
		button.className = "lk-copy";
		button.textContent = "Copy";
		button.addEventListener("click", async () => {
			await navigator.clipboard.writeText(code.innerText.replace(/\n$/, ""));
			button.textContent = "Copied";
			button.setAttribute("data-copied", "");
			setTimeout(() => {
				button.textContent = "Copy";
				button.removeAttribute("data-copied");
			}, 1600);
		});
		figure.append(button);
	}

	// Tabs --------------------------------------------------------------------

	const tabGroups = [...document.querySelectorAll("[data-lk-tabs]")];

	/** @param {string} label @param {boolean} remember */
	const selectTab = (label, remember) => {
		for (const group of tabGroups) {
			const tabs = [...group.querySelectorAll(":scope > .lk-tab")];
			if (!tabs.some((tab) => tab.dataset.label === label)) continue;

			for (const [index, tab] of tabs.entries()) {
				const selected = tab.dataset.label === label;
				tab.hidden = !selected;
				const button = group.querySelectorAll(".lk-tab-button")[index];
				button?.setAttribute("aria-selected", String(selected));
				button?.setAttribute("tabindex", selected ? "0" : "-1");
			}
		}

		if (remember) save("lk-tab", label);
	};

	for (const [groupIndex, group] of tabGroups.entries()) {
		const tabs = [...group.querySelectorAll(":scope > .lk-tab")];
		const list = document.createElement("div");
		list.className = "lk-tab-list";
		list.setAttribute("role", "tablist");

		for (const [index, tab] of tabs.entries()) {
			const id = `lk-tab-${groupIndex}-${index}`;
			const button = document.createElement("button");
			button.type = "button";
			button.className = "lk-tab-button";
			button.id = `${id}-button`;
			button.textContent = tab.dataset.label || "";
			button.setAttribute("role", "tab");
			button.setAttribute("aria-controls", id);
			button.addEventListener(
				"click",
				() => selectTab(tab.dataset.label || "", true),
			);
			button.addEventListener("keydown", (event) => {
				const step = event.key === "ArrowRight"
					? 1
					: event.key === "ArrowLeft"
					? -1
					: 0;
				if (!step) return;
				const next = tabs[(index + step + tabs.length) % tabs.length];
				selectTab(next.dataset.label || "", true);
				list.children[tabs.indexOf(next)]?.focus();
			});
			list.append(button);

			tab.id = id;
			tab.setAttribute("role", "tabpanel");
			tab.setAttribute("aria-labelledby", button.id);
		}

		group.prepend(list);
		group.setAttribute("data-ready", "");
		selectTab(tabs[0]?.dataset.label || "", false);
	}

	const rememberedTab = load("lk-tab");
	if (rememberedTab) selectTab(rememberedTab, false);

	// On this page ------------------------------------------------------------

	const tocLinks = new Map(
		[...document.querySelectorAll(".lk-toc a")].map((link) => [
			decodeURIComponent(link.getAttribute("href")?.slice(1) || ""),
			link,
		]),
	);
	const headings = [...tocLinks.keys()]
		.map((id) => document.getElementById(id))
		.filter((heading) => heading !== null);

	if (headings.length > 0 && "IntersectionObserver" in window) {
		const visible = new Set();
		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (entry.isIntersecting) visible.add(entry.target);
					else visible.delete(entry.target);
				}

				const current = headings.find((heading) => visible.has(heading)) ||
					[...headings].reverse().find((heading) =>
						heading.getBoundingClientRect().top < 0
					);

				for (const [id, link] of tocLinks) {
					if (current?.id === id) link.setAttribute("aria-current", "true");
					else link.removeAttribute("aria-current");
				}
			},
			{ rootMargin: "-64px 0px -55% 0px" },
		);
		for (const heading of headings) observer.observe(heading);
	}

	// Search ------------------------------------------------------------------

	const dialog =
		/** @type {HTMLDialogElement | null} */ (document.querySelector(
			"[data-lk-search]",
		));
	const openButton = document.querySelector("[data-lk-search-open]");
	const input = dialog?.querySelector("input");
	const results = dialog?.querySelector(".lk-search-results");

	if (dialog && openButton && input && results && "showModal" in dialog) {
		openButton.hidden = false;

		/** @type {Promise<Array<{page: string, heading: string | null, url: string, text: string}>> | null} */
		let index = null;
		let selected = 0;

		const loadIndex = () => {
			index ??= fetch(`${root}_leptoskit/search.json`).then((response) =>
				response.json()
			);
			return index;
		};

		/** @param {string} text */
		const escape = (text) =>
			text.replace(/[&<>"]/g, (character) => `&#${character.charCodeAt(0)};`);

		/** @param {string} text @param {string[]} terms */
		const mark = (text, terms) => {
			let marked = escape(text);
			for (const term of terms) {
				const pattern = new RegExp(
					`(${escape(term).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`,
					"gi",
				);
				marked = marked.replace(pattern, "<mark>$1</mark>");
			}
			return marked;
		};

		/** @param {string} text @param {string[]} terms */
		const excerpt = (text, terms) => {
			const lower = text.toLowerCase();
			const first = Math.min(
				...terms.map((term) => lower.indexOf(term)).filter((at) => at >= 0),
			);
			const start = Number.isFinite(first) ? Math.max(0, first - 40) : 0;
			return (start > 0 ? "…" : "") + text.slice(start, start + 160);
		};

		const highlight = () => {
			for (const [position, item] of [...results.children].entries()) {
				item.setAttribute("aria-selected", String(position === selected));
				if (position === selected) item.scrollIntoView({ block: "nearest" });
			}
		};

		const search = async () => {
			const query = input.value.trim().toLowerCase();
			const terms = query.split(/\s+/).filter(Boolean);
			if (terms.length === 0) {
				results.replaceChildren();
				return;
			}

			const entries = await loadIndex();
			if (input.value.trim().toLowerCase() !== query) return;

			const scored = [];
			for (const entry of entries) {
				const page = entry.page.toLowerCase();
				const heading = (entry.heading || "").toLowerCase();
				const text = entry.text.toLowerCase();
				let score = 0;
				for (const term of terms) {
					const hit = (heading.includes(term) ? 6 : 0) +
						(page.includes(term) ? 4 : 0) + (text.includes(term) ? 1 : 0);
					if (hit === 0) {
						score = 0;
						break;
					}
					score += hit;
				}
				if (score > 0) scored.push({ entry, score });
			}
			scored.sort((a, b) => b.score - a.score);

			selected = 0;
			if (scored.length === 0) {
				const empty = document.createElement("li");
				empty.className = "lk-search-empty";
				empty.textContent = `Nothing matches “${input.value.trim()}”.`;
				results.replaceChildren(empty);
				return;
			}

			results.innerHTML = scored
				.slice(0, 12)
				.map(({ entry }) => {
					const title = entry.heading
						? `${mark(entry.heading, terms)} <span class="lk-result-page">· ${
							escape(entry.page)
						}</span>`
						: mark(entry.page, terms);
					return `<li role="option"><a href="${root}${
						escape(entry.url)
					}"><span class="lk-result-title">${title}</span><span class="lk-result-text">${
						mark(excerpt(entry.text, terms), terms)
					}</span></a></li>`;
				})
				.join("");
			highlight();
		};

		const open = () => {
			if (dialog.open) return;
			dialog.showModal();
			input.select();
			loadIndex();
		};

		openButton.addEventListener("click", open);
		input.addEventListener("input", search);
		input.addEventListener("keydown", (event) => {
			const count = results.querySelectorAll("a").length;
			if (event.key === "ArrowDown" || event.key === "ArrowUp") {
				event.preventDefault();
				if (count === 0) return;
				selected = (selected + (event.key === "ArrowDown" ? 1 : -1) + count) %
					count;
				highlight();
			} else if (event.key === "Enter") {
				const link = results.querySelectorAll("a")[selected];
				if (link) {
					event.preventDefault();
					location.href = link.href;
				}
			}
		});
		dialog.addEventListener("click", (event) => {
			if (event.target === dialog) dialog.close();
		});
		results.addEventListener("click", (event) => {
			if (event.target instanceof Element && event.target.closest("a")) {
				dialog.close();
			}
		});

		document.addEventListener("keydown", (event) => {
			const typing = event.target instanceof HTMLElement &&
				(event.target.isContentEditable ||
					/^(INPUT|TEXTAREA|SELECT)$/.test(event.target.tagName));
			if (
				(event.key === "/" && !typing) ||
				(event.key === "k" && (event.metaKey || event.ctrlKey))
			) {
				event.preventDefault();
				open();
			}
		});
	}

	// Live reload -------------------------------------------------------------

	if ("liveReload" in html.dataset) {
		/** @param {string | null} seen */
		const poll = async (seen) => {
			try {
				const response = await fetch(
					`${root}_leptoskit/live?since=${seen ?? 0}`,
					{ cache: "no-store" },
				);
				const generation = await response.text();
				if (seen !== null && generation !== seen) {
					location.reload();
					return;
				}
				poll(generation);
			} catch {
				setTimeout(() => poll(seen), 1000);
			}
		};
		poll(null);
	}
})();
