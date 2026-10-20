{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

let
  currentDir = builtins.dirOf __curPos.file;
  extra = inputs.ifiokjr-nixpkgs.packages.${pkgs.stdenv.hostPlatform.system};
in
{
  # The macOS frameworks come from the host Xcode toolchain; the removed
  # `darwin.apple_sdk_11_0` stub must not be pulled in implicitly.
  apple.sdk = null;


  packages =
    with pkgs;
    [
      binaryen
      cargo-binstall
      cargo-deny
      cargo-nextest
      cargo-run-bin
      chromedriver
      curl
      dprint
      geckodriver
      gh
      git
      gitleaks
      hyperfine
      jq
      libiconv
      nixfmt-rfc-style
      openssl
      rustup
      shfmt
      taplo
      unzip
      zip
      zizmor
    ]
    ++ lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
      pkgs.coreutils
    ];

  enterShell = ''
    # Keep shell entry fast. Only bootstrap missing toolchains here; explicit
    # updates happen via install/update tasks instead of every shell.
    export PATH="$DEVENV_PROFILE/bin:$PATH"
  '';

  # disable dotenv since it breaks the variable interpolation supported by `direnv`
  dotenv.disableHint = true;

  tasks = {
    "rustfmt:nightly" = {
      # The bare `rustup toolchain install` resolves rust-toolchain.toml and
      # downloads the pinned toolchain serially, before anything else can ask
      # for it: without this, dprint's parallel leptosfmt workers each invoke
      # the rustup shim at once and race their downloads on a cold CI runner.
      exec = ''
        rustup toolchain install
        rustup toolchain install nightly --component rustfmt --force
      '';
      before = [ "devenv:enterShell" ];
    };
  };

  git-hooks = {
    hooks = {
      "secrets:commit" = {
        enable = true;
        verbose = true;
        pass_filenames = true;
        name = "secrets";
        description = "Scan staged changes for leaked secrets with gitleaks.";
        entry = "${pkgs.gitleaks}/bin/gitleaks protect --staged --verbose --redact";
        stages = [ "pre-commit" ];
      };
      "lint:format" = {
        enable = true;
        verbose = true;
        pass_filenames = true;
        name = "lint:format";
        description = "Run workspace autofixes before commit and restage the results.";
        entry = "${config.env.DEVENV_PROFILE}/bin/fix:format";
        stages = [ "pre-commit" ];
      };
      "lint:push" = {
        enable = true;
        verbose = true;
        pass_filenames = false;
        name = "lint:push";
        description = "Run the local CI lint rules and test suite before push.";
        entry = "${config.env.DEVENV_PROFILE}/bin/lint:push";
        stages = [ "pre-push" ];
      };
    };
  };

  scripts = {
    "leptoskit:dev" = {
      exec = ''
        set -euo pipefail
        cargo run --quiet --package leptoskit_cli --bin leptoskit -- "$@"
      '';
      description = "The dev build of the `leptoskit` executable";
      binary = "bash";
    };
    "leptoskit" = {
      exec = ''
        set -euo pipefail
        cargo run --quiet --release --package leptoskit_cli --bin leptoskit -- "$@"
      '';
      description = "The release build of the `leptoskit` executable";
      binary = "bash";
    };

    # Wrappers around the `cargo bin` managed executables.
    leptosfmt = {
      exec = ''
        set -euo pipefail
        cargo bin leptosfmt $@
      '';
      description = "The `leptosfmt` executable";
      binary = "bash";
    };
    "wasm-bindgen" = {
      exec = ''
        set -euo pipefail
        cargo bin wasm-bindgen $@
      '';
      description = "The `wasm-bindgen` executable";
      binary = "bash";
    };
    "wasm-bindgen-test-runner" = {
      exec = ''
        set -euo pipefail
        cargo bin wasm-bindgen-test-runner $@
      '';
      description = "The `wasm-bindgen-test-runner` executable";
      binary = "bash";
    };

    "install:all" = {
      exec = ''
        set -euo pipefail
        install:cargo:bin
        install:monochange
        cargo fetch --locked
      '';
      description = "Install all pinned tools and fetch locked dependencies.";
      binary = "bash";
    };
    "install:monochange" = {
      exec = ''
        set -euo pipefail
        # monochange is not in nixpkgs; binstall pulls the pinned release
        # binary, and the fallback covers a cache miss on fresh machines.
        cargo binstall --no-confirm monochange@0.19.0 || cargo install monochange --version 0.19.0 --locked
      '';
      description = "Install the monochange release tool.";
      binary = "bash";
    };
    "install:cargo:bin" = {
      exec = ''
        set -euo pipefail
        cargo bin --install
      '';
      description = "Install cargo binaries locally.";
      binary = "bash";
    };
    "install:targets" = {
      exec = ''
        set -euo pipefail
        rustup target add wasm32-unknown-unknown wasm32-wasip1
      '';
      description = "Install the wasm compilation targets.";
      binary = "bash";
    };
    "update:deps" = {
      exec = ''
        set -euo pipefail
        cargo update
        devenv update
      '';
      description = "Update dependencies.";
      binary = "bash";
    };

    "build:all" = {
      exec = ''
        set -euo pipefail
        if [ -z "''${CI:-}" ]; then
          echo "Building project locally"
          cargo build --workspace --all-features
        else
          echo "Building in CI"
          cargo build --workspace --all-features --locked
        fi
      '';
      description = "Build all crates with all features activated.";
      binary = "bash";
    };
    "build:dist" = {
      exec = ''
        set -euo pipefail
        echo "Building with dist profile (LTO, codegen-units=1, strip)"
        cargo build --workspace --all-features --locked --profile dist
      '';
      description = "Build the release binary with the dist profile.";
      binary = "bash";
    };
    "build:example" = {
      exec = ''
        set -euo pipefail
        cargo run --release --package counter_example -- "$@"
      '';
      description = "Build and run the counter example: a headless native UI shipped as a code push release.";
      binary = "bash";
    };

    "test:all" = {
      exec = ''
        set -euo pipefail
        test:cargo
        test:docs
      '';
      description = "Run all tests.";
      binary = "bash";
    };
    "test:cargo" = {
      exec = ''
        set -euo pipefail
        cargo nextest run --workspace --all-features
      '';
      description = "Run cargo tests with nextest.";
      binary = "bash";
    };
    "test:docs" = {
      exec = ''
        set -euo pipefail
        cargo test --doc --workspace --all-features
      '';
      description = "Run documentation tests.";
      binary = "bash";
    };
    "test:browser" = {
      exec = ''
        set -euo pipefail
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package leptos_testing_library
      '';
      description = "Run the browser (wasm-bindgen) test suite with chromedriver.";
      binary = "bash";
    };
    "coverage:all" = {
      exec = ''
        set -euo pipefail
        mkdir -p target/coverage
        cargo llvm-cov clean --workspace
        cargo llvm-cov nextest --workspace --all-features --lib --tests --no-report
        cargo llvm-cov report --summary-only --fail-under-lines 70
        cargo llvm-cov report --lcov --output-path target/coverage/lcov.info
      '';
      description = "Run workspace coverage and enforce a 70% line-coverage floor.";
      binary = "bash";
    };

    "snapshot:accept" = {
      exec = ''
        set -euo pipefail
        cargo insta accept --workspace --all-features
      '';
      description = "Accept all pending insta snapshots.";
      binary = "bash";
    };
    "snapshot:review" = {
      exec = ''
        set -euo pipefail
        cargo insta review --workspace --all-features
      '';
      description = "Review pending insta snapshots interactively.";
      binary = "bash";
    };

    "fix:all" = {
      exec = ''
        set -euo pipefail
        fix:clippy
        fix:format
      '';
      description = "Fix all autofixable problems.";
      binary = "bash";
    };
    "fix:format" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        dprint fmt --config "$repo_root/dprint.json"
      '';
      description = "Format files with dprint.";
      binary = "bash";
    };
    "fix:clippy" = {
      exec = ''
        set -euo pipefail
        cargo clippy --workspace --fix --allow-dirty --allow-staged --all-features --all-targets
      '';
      description = "Fix clippy lints for rust.";
      binary = "bash";
    };

    "lint:all" = {
      exec = ''
        set -euo pipefail

        run_step() {
          local name="$1"
          shift
          echo "Currently running: $name"
          "$@"
        }

        run_step "lint:clippy" lint:clippy
        run_step "lint:format" lint:format
        run_step "lint:workflows" lint:workflows
        run_step "lint:releases" lint:releases
        run_step "deny:check" deny:check
      '';
      description = "Run all checks.";
      binary = "bash";
    };
    "lint:push" = {
      exec = ''
        set -euo pipefail

        run_step() {
          local name="$1"
          shift
          echo "Currently running: $name"
          "$@"
        }

        # Sibling scripts are addressed absolutely so the hook also works
        # outside `devenv shell`, where the profile bin dir is not on PATH.
        run_step "gitleaks detect" ${pkgs.gitleaks}/bin/gitleaks detect --verbose --redact
        run_step "lint:clippy" ${currentDir}/.devenv/profile/bin/lint:clippy
        run_step "lint:format" ${currentDir}/.devenv/profile/bin/lint:format
        run_step "test:cargo" ${currentDir}/.devenv/profile/bin/test:cargo
        run_step "lint:workflows" ${currentDir}/.devenv/profile/bin/lint:workflows
        run_step "lint:releases" ${currentDir}/.devenv/profile/bin/lint:releases
        run_step "deny:check" ${currentDir}/.devenv/profile/bin/deny:check
      '';
      description = "Used for the pre push checks";
      binary = "bash";
    };
    "lint:clippy" = {
      exec = ''
        set -euo pipefail
        # Treat all compiler and clippy warnings as errors so warning-only regressions never
        # make it into CI or a pushed branch.
        cargo clippy --workspace --all-features --all-targets -- -D warnings
      '';
      description = "Check that all rust lints are passing with warnings denied.";
      binary = "bash";
    };
    "lint:format" = {
      exec = ''
        set -euo pipefail
        ${pkgs.dprint}/bin/dprint check
      '';
      description = "Check that all files are formatted.";
      binary = "bash";
    };
    "lint:workflows" = {
      exec = ''
        set -euo pipefail
        zizmor --config "''${DEVENV_ROOT:-$(pwd)}/.zizmor.yml" .github/workflows/
      '';
      description = "Scan GitHub Actions workflows for security vulnerabilities with zizmor.";
      binary = "bash";
    };
    "lint:releases" = {
      exec = ''
        set -euo pipefail
        monochange step validate
        monochange check
      '';
      description = "Validate the release configuration and changesets.";
      binary = "bash";
    };
    "release:preview" = {
      exec = ''
        set -euo pipefail
        monochange run release --format markdown
      '';
      description = "Preview the next release without committing anything.";
      binary = "bash";
    };
    "release:publish-check" = {
      exec = ''
        set -euo pipefail
        monochange run publish-check
      '';
      description = "Dry-run the publish path and report what would ship.";
      binary = "bash";
    };
    "deny:check" = {
      exec = ''
        set -euo pipefail
        cargo deny --workspace --all-features check
      '';
      description = "Run cargo-deny advisories, licenses, bans, and sources checks.";
      binary = "bash";
    };

    "ios_sim_run" = {
      exec = ''
        set -euo pipefail
        exec bash "$DEVENV_ROOT/setup/scripts/ios_probe_run.sh"
      '';
      description = "Run the platform probe on the iOS simulator.";
      binary = "bash";
    };
    "android_emulator_run" = {
      exec = ''
        set -euo pipefail
        exec bash "$DEVENV_ROOT/setup/scripts/android_probe_run.sh"
      '';
      description = "Run the platform probe on an Android device or emulator.";
      binary = "bash";
    };
    "bench:cli" = {
      exec = ''
        set -euo pipefail
        hyperfine --warmup 2 \
          'cargo run --quiet --release --package leptoskit_cli --bin leptoskit -- --version'
      '';
      description = "Benchmark the CLI startup overhead with hyperfine.";
      binary = "bash";
    };
    "bench:build" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        fixture="$(bash "$repo_root/setup/scripts/bench_fixture.sh")"
        cli="$repo_root/target/release/leptoskit"
        if [ ! -x "$cli" ]; then
          echo "building the release CLI first..."
          cargo build --release --package leptoskit_cli --bin leptoskit
        fi

        # First run populates cargo's cache so the benchmark measures the
        # leptoskit pipeline, not a cold rustc.
        (cd "$fixture" && "$cli" build --out dist >/dev/null 2>&1 || true)

        hyperfine --shell default --warmup 1 \
          -n "no-change rebuild" \
          "cd '$fixture' && '$cli' build --out dist"

        # The incremental run restores a pristine copy of the client source
        # before every iteration, then appends one unique line, so each
        # iteration measures exactly one source change and a failed build
        # cannot leave the edit behind.
        client_src="$fixture/crates/fixture_client/src/lib.rs"
        pristine="$(mktemp)"
        cp "$client_src" "$pristine"
        trap 'cp "$pristine" "$client_src"; rm -f "$pristine"' EXIT

        hyperfine --shell bash --warmup 0 --runs 5 \
          --prepare "cp '$pristine' '$client_src' && printf '// touch %s-%s\\n' \"\$\$\" \"\$RANDOM\" >> '$client_src'" \
          -n "one-line source change" \
          "cd '$fixture' && '$cli' build --out dist"
      '';
      description = "Benchmark cached and incremental web builds with hyperfine.";
      binary = "bash";
    };

    "docs:check" = {
      exec = ''
        set -euo pipefail
        dprint check readme.md docs/
      '';
      description = "Check that documentation files are formatted.";
      binary = "bash";
    };

    "setup:vscode" = {
      exec = ''
        set -euo pipefail
        rm -rf .vscode
        cp -r $DEVENV_ROOT/setup/editors/vscode .vscode
      '';
      description = "Setup the environment for vscode.";
      binary = "bash";
    };
    "setup:helix" = {
      exec = ''
        set -euo pipefail
        rm -rf .helix
        cp -r $DEVENV_ROOT/setup/editors/helix .helix
      '';
      description = "Setup for the helix editor.";
      binary = "bash";
    };
    "setup:ci" = {
      exec = ''
        set -euo pipefail
        # update github ci path
        echo "$DEVENV_PROFILE/bin" >> $GITHUB_PATH

        # update github ci environment
        echo "DEVENV_PROFILE=$DEVENV_PROFILE" >> $GITHUB_ENV

        # prepend common compilation lookup paths; the `:-` defaults matter
        # because Linux runners do not export several of these, and `set -u`
        # treats the unbound expansion as a failure.
        echo "PKG_CONFIG_PATH=''${PKG_CONFIG_PATH:-}" >> $GITHUB_ENV
        echo "LD_LIBRARY_PATH=''${LD_LIBRARY_PATH:-}" >> $GITHUB_ENV
        echo "LIBRARY_PATH=''${LIBRARY_PATH:-}" >> $GITHUB_ENV
        echo "C_INCLUDE_PATH=''${C_INCLUDE_PATH:-}" >> $GITHUB_ENV

        # these provide shell completions / default config options
        echo "XDG_DATA_DIRS=''${XDG_DATA_DIRS:-}" >> $GITHUB_ENV
        echo "XDG_CONFIG_DIRS=''${XDG_CONFIG_DIRS:-}" >> $GITHUB_ENV

        echo "DEVENV_DOTFILE=$DEVENV_DOTFILE" >> $GITHUB_ENV
        echo "DEVENV_ROOT=$DEVENV_ROOT" >> $GITHUB_ENV
        echo "DEVENV_STATE=$DEVENV_STATE" >> $GITHUB_ENV
      '';
      description = "Setup devenv for GitHub Actions";
      binary = "bash";
    };
  };
}
