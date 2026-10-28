import { assertEquals } from "jsr:@std/assert@1";
import { xcodeEnvironment } from "../scripts/arena/run.ts";

Deno.test("Xcode builds drop the nix toolchain and put Xcode's tools first", () => {
	const environment = xcodeEnvironment({
		PATH:
			"/nix/store/abc-devenv-profile/bin:/Users/me/.cargo/bin:/usr/bin:/bin",
		CC: "clang",
		LD: "ld",
		NIX_CFLAGS_COMPILE: "-isystem /nix/store/x/include",
		MACOSX_DEPLOYMENT_TARGET: "14.0",
		DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
		HOME: "/Users/me",
	});
	assertEquals(environment, {
		PATH:
			"/usr/bin:/Users/me/.cargo/bin:/bin:/nix/store/abc-devenv-profile/bin",
		DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
		HOME: "/Users/me",
	});
});
