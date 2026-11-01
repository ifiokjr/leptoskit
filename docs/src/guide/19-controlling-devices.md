# Controlling devices, apps, and browsers

Testing tells you a build works on your machine. Building a product asks a different question: _what is on the screen of every device on my desk right now?_ An agent fixing a layout bug needs to boot a simulator, launch the app, tap through the flow, read what actually rendered, and see the logs — on an iPhone, an Android phone, a desktop app, and the website, in one loop.

`leptoskit device` answers that. One control document names a target and lists ordered steps; the same document runs from shell flags, from JSON piped into the CLI, or through an MCP server that agents connect to. Every step reports what it did and where its screenshots and logs landed.

## What is attached

`leptoskit device list` reads the machine directly — `simctl` for simulators, `devicectl` for connected iPhones, `adb` for Android devices and emulators and installed AVDs, plus the desktop and every browser whose driver is installed:

```sh
leptoskit device list            # a table for humans
leptoskit device list --json     # the same inventory for scripts
```

## One command at a time

Every action command prints a JSON outcome and exits non-zero when its step failed, so they compose in scripts and shell loops:

```sh
leptoskit device boot "278DD3BE-CDCE-4D60-B5CE-A998A979A078"   # a simulator UDID or AVD name
leptoskit device screenshot --device SM02E4060324957           # the whole screen, no driver needed
leptoskit device start-app --device SM02E4060324957 --identifier dev.example.app
leptoskit device tap --device emulator-5554 --app dev.example.app --locator-text "Sign in"
leptoskit device expect-text --device emulator-5554 --app dev.example.app --locator-id title --text "Welcome"
leptoskit device logs --device SM02E4060324957 --tail 100
```

`--device` looks the id up in the inventory to pick the target kind; `--url https://example.com` targets a website; `--app` names the app a UI session attaches to (an installed identity, a path with a separator, or `desktop` for the whole screen on Windows and Linux).

## One document for whole journeys

The unit of composition is the [control document](https://ifiokjr.github.io/leptoskit/schemas/control.schema.json): a target and ordered steps, stopping at the first failure. `device run` reads it from a file or standard input:

```json
{
	"schema_version": 1,
	"target": {
		"kind": "ios_simulator",
		"device": "278DD3BE-CDCE-4D60-B5CE-A998A979A078",
		"application": { "kind": "identifier", "value": "dev.example.app" }
	},
	"steps": [
		{ "action": "boot" },
		{ "action": "start_app", "identifier": "dev.example.app" },
		{ "action": "tap", "locator": { "by": "text", "value": "Sign in" } },
		{
			"action": "expect_visible",
			"locator": { "by": "accessibility_id", "value": "email" }
		},
		{
			"action": "enter_text",
			"locator": { "by": "accessibility_id", "value": "email" },
			"text": "a@example.com"
		},
		{ "action": "screenshot", "label": "sign-in filled" },
		{ "action": "logs", "minutes": 2 }
	]
}
```

```sh
leptoskit device run --from journey.json
cat journey.json | leptoskit device run --from -
```

The reply reports every step with its status, any text it read, and its artifact paths; screenshots are real pixels and sources are the real accessibility tree or DOM.

### The steps

Driver steps work through one W3C session per run: `tap`, `enter_text`, `type_text`, `press_key`, `swipe`, `expect_visible`, `expect_absent`, `expect_text`, `alert_text`, `accept_alert`, `dismiss_alert`, `screenshot`, `source`, `native_command`, and `perform_actions` for raw W3C action sequences. `locator` picks controls by `accessibility_id`, `text`, `id`, `xpath` everywhere, or `css` on websites.

Device steps need no driver at all: `boot` and `shutdown` manage simulators and emulators, `start_emulator` boots an AVD detached and waits for it, `start_app` and `stop_app` launch and terminate installed apps, `device_screenshot` captures the whole screen, `device_tap`, `device_swipe`, and `device_key` inject input on Android, and `logs` reads recent `logcat` or unified logs.

### What each target needs

| Target          | Id             | UI steps need                             | Device steps work                       |
| --------------- | -------------- | ----------------------------------------- | --------------------------------------- |
| `ios_simulator` | simulator UDID | Appium with XCUITest                      | boot, shutdown, apps, screenshot, logs  |
| `ios_device`    | device UDID    | Appium with XCUITest                      | apps, (screenshot via the UI session)   |
| `android`       | adb serial     | Appium with UI Automator 2                | everything, including input and logs    |
| `macos`         | —              | Appium with Mac2, `--app` bundle id       | none; drive the app through the session |
| `windows`       | —              | Appium driver, `--app` or `--desktop`     | none; drive the app through the session |
| `linux`         | —              | KDE AT-SPI driver, `--app` or `--desktop` | none; drive the app through the session |
| `web`           | —              | the browser's own driver, spawned         | none; the session covers the page       |

Native UI sessions expect an Appium server at `LEPTOSKIT_APPIUM_ENDPOINT` (default `http://127.0.0.1:4723`) — the same provisioned drivers the native tests use. Website sessions spawn `chromedriver`, `geckodriver`, or `safaridriver` themselves (Safari needs "Allow remote automation" enabled once in Safari's Developer settings), run headless unless `headless` is `false`, and clean both the browser and the driver up when the run ends. Device steps shell out to the platform tools directly, so they work with no Appium server at all.

HarmonyOS devices cannot be driven yet; the experimental ArkUI host has no automation driver.

## Agents: MCP

`leptoskit mcp serve` speaks the Model Context Protocol on standard input and output, the way MCP clients launch servers:

```json
{
	"mcpServers": {
		"leptoskit": { "command": "leptoskit", "args": ["mcp", "serve"] }
	}
}
```

Three tools cover the surface: `list_devices` reports the inventory, `control` runs a whole document (its JSON Schema ships with the tool, so clients validate journeys before sending them), and `screenshot` returns the capture as image content with its saved path, so vision-capable agents see the screen inline.

`leptoskit mcp http --port 7331` accepts the identical JSON-RPC messages as POST bodies on loopback — no MCP client required:

```sh
curl -s http://127.0.0.1:7331/ -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
  "params":{"name":"control","arguments":{"target":{"kind":"web","url":"https://example.com"},
  "steps":[{"action":"screenshot","label":"landing"}]}}}'
```

The same document reaches all three surfaces: flags build one for a single command, `device run` takes the file, and the MCP tools take it as tool arguments.

## Agents: debugging a running app

A screenshot shows what went wrong on screen, but not why: which node is too wide, which class it has, which line built it, or what the query cache holds. `leptoskit dev` starts a devtools hub, and apps running with leptoskit's devtools report their element tree, layout, queries, and named state to it. The same `leptoskit mcp` server reads them through eight more tools:

| Tool                | What it does                                                                                                 |
| ------------------- | ------------------------------------------------------------------------------------------------------------ |
| `devtools_sessions` | Lists the apps reporting to the hub: id, app, platform, device, and what each can show.                      |
| `inspect_tree`      | Prints the element tree one node per line, with kind, text, classes, frame, and source line, to a `depth`.   |
| `inspect_node`      | Returns one node as JSON, with its match key, parent, source, frame, classes, text, and children.            |
| `explain_styles`    | Returns a node's active classes, and the declarations they resolved to when the app sends them.              |
| `query_cache`       | Returns the query cache and mutations, or one `key`.                                                         |
| `navigate`          | Opens a route in the app, such as `/settings`.                                                               |
| `highlight`         | Highlights a node; with `all_sessions`, the same node on every platform the app is running on, by match key. |
| `set_state`         | Sets a piece of named state the app exposes to the devtools. On native, a hot reload resets it.              |

For example, with the web page and the iOS simulator both running under `leptoskit dev`, an agent asks for the iOS tree and then highlights a row everywhere:

```json
{ "name": "inspect_tree", "arguments": { "session": "ios", "depth": 3 } }
{ "name": "highlight", "arguments": { "session": "ios", "node": 42, "all_sessions": true } }
```

`session` takes a session id or the start of one, a platform such as `web` or `ios`, or the app's name. Leave it out when only one app is reporting; when several are, the tool lists them. Run the MCP server from the project: the tools find the hub through `.leptoskit/devtools.json`, which `leptoskit dev` writes, and say "no hub is running — start `leptoskit dev`" when there is none.
