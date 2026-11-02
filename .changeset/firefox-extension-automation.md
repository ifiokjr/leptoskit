---
"release": minor
---

# Develop and test extensions in Firefox

Select Firefox with `--browser firefox` when running `leptoskit dev` or `leptoskit test extension`. Development builds use a temporary add-on installed through WebDriver BiDi, reload it after a rebuild, and check page surfaces, background startup, and content script injection. `leptoskit doctor` reports Firefox and geckodriver.
