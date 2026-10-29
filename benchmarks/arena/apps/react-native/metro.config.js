const path = require("node:path");
const { getDefaultConfig, mergeConfig } = require("@react-native/metro-config");

const tierDir = path.join(__dirname, "tier");

/**
 * The tier shell in `tier/` is its own project with its own node_modules, so
 * Metro never crawls it when bundling the scenario app.
 *
 * @type {import('@react-native/metro-config').MetroConfig}
 */
const config = {
	resolver: {
		blockList: [
			new RegExp(`^${tierDir.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/.*`),
		],
	},
};

module.exports = mergeConfig(getDefaultConfig(__dirname), config);
