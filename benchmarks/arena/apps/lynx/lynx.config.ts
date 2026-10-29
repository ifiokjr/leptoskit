import { pluginReactLynx } from "@lynx-js/react-rsbuild-plugin";
import { defineConfig } from "@lynx-js/rspeedy";

// One source, two outputs: `dist/main.lynx.bundle` for the iOS and Android
// hosts (copied into their app assets) and `dist/main.web.bundle` for
// `@lynx-js/web-core` on the web.
export default defineConfig({
	plugins: [pluginReactLynx()],
	environments: {
		lynx: {},
		web: {},
	},
});
