// A small enhancement layer over the server-rendered LeptosKit component tree.
// Credentials are exchanged for HttpOnly cookies and never saved in JS storage.
const find = (root, id) => root.querySelector(`[data-testid="${id}"]`);
const notice = (message) => {
	const node = find(document, "notice");
	if (node) node.textContent = message;
};
const key = find(document, "owner-key");
if (key) {
	key.type = "password";
	key.autocomplete = "off";
}
let pending = false;
async function api(path, body) {
	const response = await fetch(path, {
		method: body === undefined ? "GET" : "POST",
		credentials: "same-origin",
		redirect: "error",
		headers: { "content-type": "application/json" },
		body: body === undefined ? undefined : JSON.stringify(body),
	});
	if (!response.ok) {
		const problem = await response.json().catch(() => ({
			error: "request_failed",
		}));
		throw new Error(problem.error || "request_failed");
	}
	return response.status === 204 ? null : response.json();
}
document.addEventListener("click", async (event) => {
	const button = event.target.closest("button[data-testid]");
	if (!button || button.disabled || pending) return;
	const action = button.dataset.testid;
	if (action === "setup" || action === "maintenance") {
		button.setAttribute(
			"aria-pressed",
			button.getAttribute("aria-pressed") !== "true",
		);
		return;
	}
	pending = true;
	try {
		if (action === "login") {
			const value = key.value;
			key.value = "";
			await api("/api/v1/login", { key: value });
			location.reload();
		} else if (action === "logout") {
			await api("/api/v1/logout", {});
			location.reload();
		} else {
			const card = button.closest('[data-testid^="project-"]');
			if (!card) return;
			const id = card.dataset.testid.slice(8);
			const overview = await api("/api/v1/overview");
			const project = overview.projects.find((project) => project.id === id);
			if (!project) throw new Error("project_unavailable");
			const version = find(card, "version").value.trim();
			let intent;
			if (action.startsWith("apply-")) {
				intent = { kind: "apply", plan_id: action.slice(6) };
			} else if (action === "deploy") {
				intent = {
					kind: "deploy",
					version,
					setup: find(card, "setup").getAttribute("aria-pressed") === "true",
					maintenance:
						find(card, "maintenance").getAttribute("aria-pressed") === "true",
				};
			} else if (action === "rollback") intent = { kind: "rollback", version };
			else if (["plan", "status", "backup"].includes(action)) {
				intent = { kind: action };
			} else return;
			const operation = await api(
				`/api/v1/projects/${encodeURIComponent(id)}/operations`,
				{
					expected_revision: project.revision,
					idempotency_key: crypto.randomUUID(),
					action: intent,
				},
			);
			notice(
				"Operation queued. You can close this page; the worker will continue.",
			);
			for (;;) {
				await new Promise((resolve) => setTimeout(resolve, 1500));
				const status = await api(`/api/v1/operations/${operation.id}`);
				notice(`${status.phase}: ${status.message}`);
				if (!["queued", "running"].includes(status.phase)) {
					location.reload();
					break;
				}
			}
		}
	} catch (error) {
		notice(error.message);
	} finally {
		pending = false;
	}
});
