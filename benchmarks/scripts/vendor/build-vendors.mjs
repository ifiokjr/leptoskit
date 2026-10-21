// Bundles the comparison frameworks into the committed vendor directories.
//
// One bundle per framework, with a single inlined React copy so hooks work
// without a bundler at benchmark time: the fixture pages import straight
// from these files through an import map or a relative specifier.
//
// Run from `benchmarks/scripts/vendor` with node available:
//   npm install && npm run build
import { build } from "esbuild";
import { mkdirSync } from "node:fs";

const here = new URL(".", import.meta.url).pathname;
const reactVendor = `${here}../../apps/runtime/react/vendor`;
const rnwVendor = `${here}../../apps/runtime/react-native-web/vendor`;

mkdirSync(reactVendor, { recursive: true });
mkdirSync(rnwVendor, { recursive: true });

await build({
	entryPoints: ["react-entry.mjs"],
	bundle: true,
	minify: true,
	format: "esm",
	outfile: `${reactVendor}/react-bundle.js`,
	logLevel: "info",
});

await build({
	entryPoints: ["rnw-entry.mjs"],
	bundle: true,
	minify: true,
	format: "esm",
	outfile: `${rnwVendor}/rnw-bundle.js`,
	// `react-native-web` resolves `react-native` internally through its own
	// aliasing; keeping react external to nothing and bundling once yields a
	// single React instance shared by app and renderer.
	logLevel: "info",
});

console.log("vendored react and react-native-web bundles");
