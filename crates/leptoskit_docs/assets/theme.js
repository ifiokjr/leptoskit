// Apply the saved theme before first paint without requiring inline scripts.
try {
	const theme = localStorage.getItem("lk-theme");
	if (theme === "light" || theme === "dark") {
		document.documentElement.dataset.theme = theme;
	}
} catch {
	// Storage can be unavailable; the system colour scheme remains usable.
}
