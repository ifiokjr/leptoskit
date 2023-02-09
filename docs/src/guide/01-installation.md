# Installation

## Automatic

There are several convenience scripts to download and install leptoskit.

Using Shell (macOS and Linux):

```bash
curl -fsSL https://raw.githubusercontent.com/ifiokjr/actionify/main/setup/scripts/install.sh | sh
```

Using PowerShell (Windows):

```powershell
irm https://raw.githubusercontent.com/ifiokjr/actionify/main/setup/scripts/install.ps1 | iex
```

Using Scoop (Windows):

```powershell
scoop install deno
```

Build and install from source using Cargo:

```bash
cargo install leptoskit_cli --locked
```

Verify the installation succeeded by running the following command.

```bash
leptoskit --help

# or

lk -h
```
