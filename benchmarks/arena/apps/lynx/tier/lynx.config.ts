import { pluginReactLynx } from "@lynx-js/react-rsbuild-plugin";
import { defineConfig } from "@lynx-js/rspeedy";

// The size-tier app, with the scenario app's two outputs:
// `dist/main.lynx.bundle` for the shared iOS and Android hosts, and
// `dist/main.web.bundle` for web-core. Tier builds pick one with
// `--environment lynx` or `--environment web`.
export default defineConfig({
	plugins: [pluginReactLynx()],
	environments: {
		lynx: {},
		web: {},
	},
});
