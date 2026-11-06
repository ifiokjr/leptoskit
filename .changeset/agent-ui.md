---
"release": minor
---

# Interfaces from agents

`leptoskit_agent_ui`, also `leptoskit`'s `agent-ui` feature, renders interfaces an AI agent writes as text while your app runs. The agent answers in Markdown with components in it — `<Stat>`, `<Chart>`, `<Slider>`, `<Button>`, and more — declares variables with `<Var>` and formulas with `<Let>`, and writes `{formulas}` in text that follow the user's input without a new message. `<View>`, `<Text>`, and `<Pressable>` let it build layouts the other components don't cover. `AgentUi::view` follows a streaming answer and shows a component that is still open. Button presses reach `on_action` with the user's state, and the app decides what they do.

Documents style with Tailwind classes and colour roles such as `bg-surface` and `text-muted`, never colours. Each role reads your `[theme.colors]` entry of the same name, and its `-dark` twin in dark appearance, so documents and the built-in components match your app in light and dark without the agent knowing either; `agent_ui::Theme` renames roles and adds your own.

Sliders, switches, checkboxes, radios, and spinners are the platform's own controls, updated in place so a slider keeps its drag. `Widgets::WithoutControls` draws them from buttons for macOS, iOS, and Android shells generated before 0.3, and `Widgets::Basic` also leaves out vector paths for HarmonyOS.

Documents are data, never code: formulas run on a fixed budget, classes that would lay content over the app or scroll are refused, images load only from allowed addresses, and `Limits` bound every document. `AgentUi::reference` writes the language reference for the agent from the components and colour roles registered right now, and `AgentUi::check` lists a document's problems by line and column.
