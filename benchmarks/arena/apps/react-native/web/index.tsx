import { createRoot } from "react-dom/client";
import { App } from "../src/App";

const root = document.getElementById("root");
if (root === null) {
	throw new Error("index.html has no #root element");
}
const scenario = new URLSearchParams(location.search).get("scenario") ??
	undefined;
createRoot(root).render(<App scenario={scenario} />);
