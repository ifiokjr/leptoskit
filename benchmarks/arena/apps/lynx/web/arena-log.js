// The web half of the `ArenaLog` native module. web-core runs the bundle's
// background thread in a worker, whose console the page does not see, so
// report lines are forwarded to the page and logged there (see index.html).
export default function createArenaLog(_nativeModules, call) {
	return {
		log(line) {
			call("log", line);
		},
	};
}
