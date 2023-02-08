# leptoskit

> A toolkit for building full stack applications with [leptos](https://github.com/leptos-rs/leptos).

## Contributing

[`devenv`](https://devenv.sh/) is used to provide a reproducible development environment for this project. Follow the [getting started instructions](https://devenv.sh/getting-started/).

To automatically load the environment you should [install direnv](https://devenv.sh/automatic-shell-activation/) and then load the `direnv`.

```bash
# The security mechanism didn't allow to load the `.envrc`.
# Since we trust it, let's allow it execution.
direnv allow .
```

At this point you should see the `nix` commands available in your terminal.

To setup recommended configurations for your favorite editor run the following commands. If you're editor is missing please open a pull request to add the desired configuration.

```bash
setup:vscode # Setup vscode
setup:helix  # Setup helix configuration
```
