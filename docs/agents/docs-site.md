# Writing the public docs

The site at <https://ifiokjr.github.io/leptoskit/> is built from `docs/src` by `docs/site`, a small binary over the [`leptoskit_docs`](../../crates/leptoskit_docs) crate. `devenv shell docs:serve` serves it on port 4400 and reloads on every edit; `devenv shell docs:build` builds `docs/dist` exactly as the `docs` workflow publishes it.

## Who the site is for

The site is for people building apps with leptoskit. Every page should answer what a reader can do, what they type, and what happens on each platform. Keep these out of it:

- repository workflows: `devenv` scripts, coverage, snapshot review, release steps (put them in [`CONTRIBUTING.md`](../../CONTRIBUTING.md));
- CI job names and what they run, links to test files, and "evidence" paragraphs (put them in [platform validation](platform-validation.md) or the topic's file in this folder);
- design investigations and proposals for unbuilt APIs (keep them in this folder, like [code splitting](code-splitting.md));
- flags that only matter when developing leptoskit itself, such as `--sdk-path` and `--bridge-path`.

When a feature is limited, say so on the page, specifically: which platform, which control or property, and what happens instead. "Younger" or "expect gaps" is not specific enough.

## Writing a page

- Start a feature with the problem it solves, then a concrete example, then the details. Name the platforms an example targets and say what happens on each.
- Prefer one complete example that builds over several fragments. Examples on guide pages should compile against the current CLI scaffold.
- Use the current CLI: `leptoskit build --platform <platform>` for apps, `leptoskit native build <target>` only for simulator and architecture variants such as `ios-simulator` or `android-x86_64`.
- List every platform when listing platforms: web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental).

## What the strict build rejects

Pages are MDX-style Markdown, and the build fails on any of these, with the file, line, and column:

- a tag that is never closed: write `<br />` and `<img ... />`; HTML comments are fine;
- an unknown component, or an attribute a component never reads;
- a link that is not relative, or that points at a `.html` page instead of its `.md` source (`guide/x.md#anchor`), or at a chapter missing from `SUMMARY.md`, or at a heading that does not exist;
- a link to a local address such as `http://localhost:3000` or `http://127.0.0.1:3000`. Bare URLs in prose become links in GitHub-flavoured Markdown, so write dev-server addresses as inline code.

Don't indent the Markdown children of a component after a blank line. dprint reads indented lines as an indented code block and rewrites them. Run `dprint fmt` (or `devenv shell fix:all`) before committing; tables are aligned by it.
