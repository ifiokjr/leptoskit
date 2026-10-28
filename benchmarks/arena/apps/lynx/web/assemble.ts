// Assembles a static web target in `<project>/dist/web`: this directory's
// host page, the project's web bundle, and `@lynx-js/web-core`'s prebuilt
// client. The scenario app runs it after `rspeedy build`; the size-tier
// builds pass their generated project as the argument. Runs with Node's
// built-in TypeScript support.
//
//   node web/assemble.ts [project]   (default: the scenario app)

import {
	cpSync,
	mkdirSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { argv } from "node:process";
import { fileURLToPath } from "node:url";

const webRoot = dirname(fileURLToPath(import.meta.url));
const project = resolve(argv[2] ?? dirname(webRoot));
const out = join(project, "dist", "web");
const webCore = join(project, "node_modules", "@lynx-js", "web-core");

interface PackageManifest {
	version: string;
}

const manifest = JSON.parse(
	readFileSync(join(webCore, "package.json"), "utf8"),
) as PackageManifest;

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
cpSync(join(webCore, "dist", "client_prod", "static"), join(out, "static"), {
	recursive: true,
});
cpSync(join(project, "dist", "main.web.bundle"), join(out, "main.web.bundle"));
cpSync(join(webRoot, "arena-log.js"), join(out, "arena-log.js"));
const page = readFileSync(join(webRoot, "index.html"), "utf8")
	.replaceAll("__WEB_CORE_VERSION__", manifest.version);
writeFileSync(join(out, "index.html"), page);
console.log(`web target assembled in ${out}`);
