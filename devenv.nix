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
  # macOS 27 cannot load stripped proc-macro dylibs (rust-lang/rust#157750),
  # and cargo strips release build dependencies by default. .cargo/config.toml
  # keeps them unstripped for builds in this checkout, but `cargo install` of a
  # remote crate, which is how `cargo bin` installs the pinned tools, ignores
  # that file, so every script that can trigger such an install exports the
  # same setting. Remove both once the upstream issue is fixed.
  keepProcMacrosLoadable = "export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none";
in
{
  # The macOS frameworks come from the host Xcode toolchain (see `enterShell`);
  # the removed `darwin.apple_sdk_11_0` stub must not be pulled in implicitly.
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
      deno
      dprint
      geckodriver
      gh
      git
      gitleaks
      hyperfine
      jq
      jdk
      ktlint
      kotlin
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
      pkgs.swiftlint
    ]
    ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
      pkgs.at-spi2-core
      pkgs.dbus
      pkgs.dejavu_fonts
      pkgs.gtk4
      pkgs.pkg-config
      pkgs.xdotool
      pkgs.xvfb-run
    ];

  enterShell = ''
    # Keep shell entry fast. Only bootstrap missing toolchains here; explicit
    # updates happen via install/update tasks instead of every shell.
    export PATH="$DEVENV_PROFILE/bin:$PATH"
  ''
  + lib.optionalString pkgs.stdenv.hostPlatform.isDarwin ''
    # nix's clang wrapper links against its own bundled macOS SDK unless
    # `DEVELOPER_DIR` names another. That SDK lags the host and changes how
    # AppKit behaves for the binaries it links, so point it at Xcode, as CI does.
    xcode_developer_dir="$(/usr/bin/xcode-select --print-path 2>/dev/null || true)"
    if [ -d "$xcode_developer_dir/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk" ]; then
      export DEVELOPER_DIR="$xcode_developer_dir"
    else
      echo "warning: no Xcode macOS SDK at '$xcode_developer_dir'; native macOS builds use nix's older SDK. Run 'sudo xcode-select --switch /Applications/Xcode.app'." >&2
    fi
    unset xcode_developer_dir
  '';

  # disable dotenv since it breaks the variable interpolation supported by `direnv`
  dotenv.disableHint = true;

  tasks = {
    "rustfmt:nightly" = {
      # The bare `rustup toolchain install` resolves rust-toolchain.toml and
      # downloads the pinned toolchain serially, before anything else can ask
      # for it: without this, parallel formatter workers each invoke the
      # rustup shim at once and race their downloads on a cold CI runner.
      # `leptoskit format` runs nightly rustfmt, installed alongside.
      #
      # rustup itself does not lock across processes (rust-lang/rustup#988):
      # two shells installing the same toolchain at once leave it with files
      # neither owns, and the next install fails with "detected conflict".
      # Every worktree's shell runs this on entry, and each new nightly makes
      # them all download at once, so the installs share one machine-wide
      # lock in the rustup home.
      exec = ''
        set -euo pipefail
        rustup_home="''${RUSTUP_HOME:-$HOME/.rustup}"
        mkdir -p "$rustup_home"
        install_status=0
        ${pkgs.flock}/bin/flock --wait 900 --conflict-exit-code 75 \
          "$rustup_home/leptoskit-toolchains.lock" bash -euo pipefail -c '
            rustup toolchain install
            rustup toolchain install nightly --component rustfmt --force
          ' || install_status=$?
        if [ "$install_status" -eq 75 ]; then
          echo "another shell held the rustup install lock for 15 minutes:" \
            "$rustup_home/leptoskit-toolchains.lock" >&2
        fi
        exit "$install_status"
      '';
      before = [ "devenv:enterShell" ];
    };
    # Git keeps hooks in the common directory, so every worktree of this
    # repository runs the same hook scripts, but prek writes the absolute path
    # of the installing worktree's `.pre-commit-config.yaml` into them. The
    # last worktree to enter its shell then decides which configuration every
    # other worktree's commits and pushes run, checks and pinned tools
    # included. Git runs these hooks from the top of the working tree, so the
    # relative path upstream pre-commit writes resolves to each worktree's
    # own configuration.
    #
    # That configuration is a git-ignored link devenv writes on shell entry,
    # so a worktree that has never entered its shell has none. The hooks then
    # stop with the command that creates it, rather than letting prek fail
    # with a bare "config file not found" or skipping the checks.
    "git-hooks:per-worktree-config" = {
      exec = ''
        set -euo pipefail
        hooks="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"
        for hook in "$hooks"/*; do
          [ -f "$hook" ] && grep -q '^# File generated by prek' "$hook" || continue
          relative="$(
            sed -E 's|--config="[^"]*"|--config=".pre-commit-config.yaml"|' "$hook" |
              awk '
                /^# leptoskit: worktree configuration guard$/ { guarded = 1 }
                /^exec "\$PREK"/ && !guarded {
                  print "# leptoskit: worktree configuration guard"
                  print "if [ ! -e .pre-commit-config.yaml ]; then"
                  print "    echo \"git hook: this worktree has no .pre-commit-config.yaml; run devenv shell -- true in $(pwd) once to generate it\" >&2"
                  print "    exit 1"
                  print "fi"
                  print ""
                }
                { print }
              '
          )"
          # Compare before writing, so shells entering at the same time do not
          # rewrite the shared hooks for nothing.
          if [ "$relative" != "$(cat "$hook")" ]; then
            printf '%s\n' "$relative" >"$hook"
          fi
        done
      '';
      after = [ "devenv:git-hooks:install" ];
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

    # The repository formats its Rust with its own formatter, built from
    # this checkout: the same command as `leptoskit format`, without
    # compiling the rest of the CLI. Fixtures and snapshots hold input that
    # is deliberately unformatted or compared byte for byte (trybuild's
    # tests/ui pin compiler output by line), and the
    # benchmark apps must match the base branch's for the self comparison.
    "leptoskit:format" = {
      exec = ''
        set -euo pipefail
        cargo run --quiet --locked --package leptoskit_fmt --features cli --bin leptoskit-fmt -- \
          --exclude '**/fixtures/**' --exclude '**/snapshots/**' --exclude 'benchmarks/apps/**' --exclude '**/tests/ui/**' "$@"
      '';
      description = "Format Rust and the markup in `view!` macros with leptoskit_fmt.";
      binary = "bash";
    };

    # Wrappers around the `cargo bin` managed executables.
    # The wasm-bindgen tools run as the wasm32 cargo runner, including under
    # `cargo +nightly test`, which exports RUSTUP_TOOLCHAIN. cargo-run-bin keys
    # its cache by the active rustc, so drop that override to resolve the
    # binaries install:cargo:bin pinned instead of installing another copy
    # mid-test, without the setup step's GitHub token.
    "wasm-bindgen" = {
      exec = ''
        set -euo pipefail
        ${keepProcMacrosLoadable}
        env -u RUSTUP_TOOLCHAIN cargo bin wasm-bindgen "$@"
      '';
      description = "The `wasm-bindgen` executable";
      binary = "bash";
    };
    "wasm-bindgen-test-runner" = {
      exec = ''
        set -euo pipefail
        ${keepProcMacrosLoadable}
        env -u RUSTUP_TOOLCHAIN cargo bin wasm-bindgen-test-runner "$@"
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
        # Use monochange v0.21.0's declared nightly for its source fallback.
        # Its classification crate uses the unstable coverage_attribute feature.
        # Preserve symbols to avoid this compiler's Mach-O strip bug on macOS 27.
        cargo binstall --no-confirm --disable-strategies compile monochange@0.21.0 || {
          rustup toolchain install nightly-2026-05-08 --profile minimal
          CARGO_PROFILE_RELEASE_STRIP=none cargo +nightly-2026-05-08 install monochange --version 0.21.0 --locked
        }
      '';
      description = "Install the monochange release tool.";
      binary = "bash";
    };
    "install:cargo:bin" = {
      exec = ''
        set -euo pipefail
        ${keepProcMacrosLoadable}
        cargo bin --install
        # cargo-run-bin ignores its installer's exit status, so a failed
        # binstall (for example a timed-out release lookup, whose source
        # fallback cannot honour `--install-path`) would otherwise surface
        # later as "Process failed to start". Confirm every binary landed.
        rust_version="$(rustc --version | cut -d' ' -f2)"
        case "$rust_version" in *-nightly | *-dev) rust_version=nightly ;; esac
        bin_root="$(git rev-parse --show-toplevel)/.bin/rust-$rust_version"
        cargo metadata --no-deps --format-version 1 |
          jq -r '.metadata.bin | to_entries[] | .key as $package | .value.version as $version | (.value.bins // [$package])[] | "\($package) \($version) \(.)"' |
          while read -r package version bin; do
            if [ ! -x "$bin_root/$package/$version/bin/$bin" ]; then
              echo "cargo bin did not install $bin from $package $version" >&2
              exit 1
            fi
          done
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
    "benchmark:self" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        out_dir="''${LEPTOSKIT_BENCH_OUT:-$repo_root/target/benchmarks/self}"
        mkdir -p "$out_dir"
        deno run -A "$repo_root/benchmarks/scripts/self.ts" --out-dir "$out_dir"
        cat "$out_dir/comment.md"
      '';
      description = "Run the full base-vs-origin/main benchmark comparison and print the report.";
      binary = "bash";
    };
    "benchmark:runtime" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        deno run -A "$repo_root/benchmarks/scripts/runtime_bench.ts" "$@"
      '';
      description = "Run the browser runtime benchmark for one built dist (--app, --dist, --out).";
      binary = "bash";
    };
    "benchmark:build" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        deno run -A "$repo_root/benchmarks/scripts/build_bench.ts" "$@"
      '';
      description = "Run the build-speed benchmark (--tool name|fixture|command).";
      binary = "bash";
    };
    "benchmark:external" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        cargo build --release --package leptoskit_cli --bin leptoskit
        deno run -A "$repo_root/benchmarks/scripts/external.ts" \
          --cli "$repo_root/target/release/leptoskit" \
          --runtime-out "$repo_root/benchmarks/results/external-runtime.json" \
          --build-out "$repo_root/benchmarks/results/external-build.json" "$@"
      '';
      description = "Run every present framework benchmark into benchmarks/results/.";
      binary = "bash";
    };
    "benchmark:native" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        deno run -A "$repo_root/benchmarks/scripts/native_bench.ts" \
          --out "$repo_root/benchmarks/results/native-host.json" --subject native-host "$@"
      '';
      description = "Measure the view layer, the wasm runtime, and the QuickJS baseline on this host.";
      binary = "bash";
    };
    "benchmark:native:ios" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        udid="$(xcrun simctl list devices available | grep -m1 -oE '[0-9A-F-]{36}.*\(Booted\)' | grep -oE '[0-9A-F-]{36}' || true)"
        if [ -z "$udid" ]; then
          echo "boot an iOS simulator first (xcrun simctl boot <udid>)" >&2
          exit 1
        fi
        cargo build --release --package leptoskit_cli --bin leptoskit
        deno run -A "$repo_root/benchmarks/scripts/native_bench.ts" \
          --platform ios --device "$udid" --cli "$repo_root/target/release/leptoskit" \
          --out "$repo_root/benchmarks/results/native-ios.json" --subject native-ios "$@"
      '';
      description = "Measure the native, wasm, and JavaScript layers on the booted iOS simulator.";
      binary = "bash";
    };
    "benchmark:native:android" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        serial="''${1:-emulator-5554}"
        shift || true
        cargo build --release --package leptoskit_cli --bin leptoskit
        deno run -A "$repo_root/benchmarks/scripts/native_bench.ts" \
          --platform android --device "$serial" --cli "$repo_root/target/release/leptoskit" \
          --out "$repo_root/benchmarks/results/native-android.json" --subject native-android "$@"
      '';
      description = "Measure the native, wasm, and JavaScript layers on an Android device or emulator (serial as the first argument; --android-arch x86_64 for emulators).";
      binary = "bash";
    };
    "benchmark:size" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        cargo build --release --package leptoskit_cli --bin leptoskit
        deno run -A "$repo_root/benchmarks/scripts/size_bench.ts" \
          --cli "$repo_root/target/release/leptoskit" \
          --out "$repo_root/benchmarks/results/sizes.json" --subject sizes "$@"
      '';
      description = "Measure what a minimal app weighs on web, macOS, iOS, and Android.";
      binary = "bash";
    };
    "benchmark:docs" = {
      exec = ''
        set -euo pipefail
        benchmark:native
        benchmark:size
        benchmark:external --docs
      '';
      description = "Rerun the external and native benchmarks and regenerate the docs tables.";
      binary = "bash";
    };
    "benchmark:test" = {
      exec = ''
        set -euo pipefail
        repo_root="$(git rev-parse --show-toplevel)"
        deno test "$repo_root/benchmarks/tests/"
      '';
      description = "Run the benchmark harness unit tests.";
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
        test:harmonyos
        test:docs
      '';
      description = "Run all tests.";
      binary = "bash";
    };
    "test:cargo" = {
      exec = ''
        set -euo pipefail
        if [ "$(uname -s)" = Linux ] && [ -z "''${DISPLAY:-}" ]; then
          exec dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
            cargo nextest run --workspace --all-features
        fi
        cargo nextest run --workspace --all-features
      '';
      description = "Run cargo tests with nextest.";
      binary = "bash";
    };

    "test:harmonyos" = {
      exec = ''
        set -euo pipefail
        deno test --config crates/leptoskit_cli/src/native/templates/harmonyos/deno.json \
          crates/leptoskit_cli/src/native/templates/harmonyos/scene_test.ts
      '';
      description = "Check HarmonyOS render mutations and widget state (no device SDK required).";
      binary = "bash";
    };
    "test:harmonyos:sdk" = {
      exec = ''
        set -euo pipefail
        cargo test --locked -p leptoskit_cli --test harmonyos_sdk -- --ignored --nocapture
      '';
      description = "Build both HarmonyOS ABIs and their instrumentation HAPs with the public SDK (requires OHOS_SDK_HOME, HVIGORW, and LEPTOSKIT_HYPIUM).";
      binary = "bash";
    };
    "test:widgets:gtk" = {
      exec = ''
        set -euo pipefail
        dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
          cargo test --locked -p leptoskit_gtk --test widgets -- --test-threads=1
      '';
      description = "Exercise real GTK widgets, keyboard input, pixels, and accessibility under Xvfb.";
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
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package leptoskit_query
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package leptoskit_native --features web --test web_browser
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package leptoskit_use --features local-storage --test web_storage --test web_storage_worker
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package gallery --features web --test web
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --target wasm32-unknown-unknown --package file_routes_example --features web --test web_browser
      '';
      description = "Run the Leptos and shared-component browser suites with chromedriver.";
      binary = "bash";
    };
    "coverage:use" = {
      exec = ''
        set -euo pipefail
        mkdir -p target/coverage
        cargo llvm-cov --package leptoskit_use --all-features --tests \
          --ignore-filename-regex '/tests/|/crates/leptoskit_(native|yoga)/' --fail-under-lines 100 \
          --lcov --output-path target/coverage/leptoskit-use.lcov
      '';
      description = "Enforce complete line coverage for cross-platform use primitives.";
      binary = "bash";
    };
    "coverage:use:browser" = {
      exec = ''
        set -euo pipefail
        task_browser_root="$(git rev-parse --show-toplevel)"
        # wasm-bindgen-test 0.3.79 pins minicov 0.3.8, the LLVM 22 profiling
        # runtime (raw profile v10). LLVM 23 nightlies emit v11 profile data it
        # cannot write, so keep this toolchain on LLVM 22 until that pin moves.
        task_browser_toolchain=nightly-2026-05-08
        task_browser_output="$task_browser_root/target/coverage/use-browser"
        mkdir -p "$task_browser_output"
        rm -f "$task_browser_output"/*.profraw
        rustup toolchain install "$task_browser_toolchain" --profile minimal \
          --component llvm-tools-preview --target wasm32-unknown-unknown

        export CARGO_TARGET_DIR="$task_browser_root/target/use-browser-coverage"
        export RUSTFLAGS="''${RUSTFLAGS:-} --cfg wasm_bindgen_unstable_test_coverage"
        # wasm-bindgen-test-runner >= 0.2.107 reads LLVM_PROFILE_FILE, as Rust
        # test coverage does (%m: module, %p: process; one runner per test).
        export LLVM_PROFILE_FILE="$task_browser_output/leptoskit_use_%m_%p.profraw"
        # Scope instrumentation to this crate. Some dependencies build auxiliary
        # wasm modules which cannot link a coverage runtime.
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          PATH="${pkgs.llvmPackages.clang-unwrapped}/bin:$PATH" \
          cargo +"$task_browser_toolchain" -Zprofile-rustflags \
          --config 'profile.dev.package.leptoskit_use.rustflags=["-Cinstrument-coverage","-Zno-profiler-runtime","--emit=llvm-ir"]' \
          test --locked --package leptoskit_use --features local-storage \
          --target wasm32-unknown-unknown --test web_storage --test web_storage_worker \
          --message-format=json > "$task_browser_output/build.log"

        task_browser_binary="$(jq -Rr 'fromjson? | select(.reason == "compiler-artifact" and .target.name == "web_storage" and .executable != null) | .executable' "$task_browser_output/build.log")"
        task_worker_binary="$(jq -Rr 'fromjson? | select(.reason == "compiler-artifact" and .target.name == "web_storage_worker" and .executable != null) | .executable' "$task_browser_output/build.log")"
        test -n "$task_browser_binary"
        test -n "$task_worker_binary"
        task_browser_sysroot="$(rustc +"$task_browser_toolchain" --print sysroot)"
        task_browser_host="$(rustc +"$task_browser_toolchain" -vV | sed -n 's/^host: //p')"
        task_browser_llvm="$task_browser_sysroot/lib/rustlib/$task_browser_host/bin"
        "$task_browser_llvm/llvm-profdata" merge -sparse "$task_browser_output"/*.profraw \
          -o "$task_browser_output/coverage.profdata"
        task_browser_source="$task_browser_root/crates/leptoskit_use/src/persistent/web.rs"
        "$task_browser_llvm/llvm-cov" export "$task_browser_binary" \
          --object "$task_worker_binary" --instr-profile "$task_browser_output/coverage.profdata" \
          "$task_browser_source" > "$task_browser_output/coverage.json"
        jq -e '.data[0].totals.lines | .count > 0 and .covered == .count' "$task_browser_output/coverage.json"
        "$task_browser_llvm/llvm-cov" report "$task_browser_binary" \
          --object "$task_worker_binary" --instr-profile "$task_browser_output/coverage.profdata" \
          "$task_browser_source"
        "$task_browser_llvm/llvm-cov" export "$task_browser_binary" --format=lcov \
          --object "$task_worker_binary" --instr-profile "$task_browser_output/coverage.profdata" \
          "$task_browser_source" > "$task_browser_root/target/coverage/leptoskit-use-browser.lcov"
      '';
      description = "Enforce complete browser storage coverage in Chrome and a worker.";
      binary = "bash";
    };
    # The browser suites, instrumented, so code that only runs in a browser
    # (leptos_testing_library, the web adapter, browser storage) counts
    # toward the coverage gate. It uses coverage:use:browser's pinned nightly
    # and writes repository-relative paths, because the gate runs elsewhere.
    "coverage:browser" = {
      exec = ''
        set -euo pipefail
        root="$(git rev-parse --show-toplevel)"
        # See coverage:use:browser: minicov 0.3.8 writes LLVM 22 profiles.
        toolchain=nightly-2026-05-08
        output="$root/target/coverage/browser"
        rm -rf "$output"
        mkdir -p "$output"
        rustup toolchain install "$toolchain" --profile minimal \
          --component llvm-tools-preview --target wasm32-unknown-unknown

        export CARGO_TARGET_DIR="$root/target/browser-coverage"
        export RUSTFLAGS="''${RUSTFLAGS:-} --cfg wasm_bindgen_unstable_test_coverage"
        export LLVM_PROFILE_FILE="$output/%m_%p.profraw"
        export CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver"
        export PATH="${pkgs.llvmPackages.clang-unwrapped}/bin:$PATH"
        # Instrument only the workspace libraries under test: some dependencies
        # build auxiliary wasm modules that cannot link a coverage runtime.
        # Only wasm-bindgen-test harnesses supply that runtime, so no package
        # whose tests also build binaries can run here: cargo builds the
        # gallery's gallery, gallery-web, and gallery-ios for its tests, and
        # they fail to link against an instrumented leptoskit_native
        # (`__llvm_profile_runtime`). The gallery suite runs uninstrumented in
        # test:browser; leptoskit_native's own suite measures the web adapter.
        instrument=()
        for package in leptos_testing_library leptoskit_native leptoskit_use; do
          instrument+=(--config "profile.dev.package.$package.rustflags=[\"-Cinstrument-coverage\",\"-Zno-profiler-runtime\",\"--emit=llvm-ir\"]")
        done
        # Build first to collect the test modules from cargo's JSON messages
        # (compiler errors still print readably), then run the same build with
        # the tests' output on screen.
        suite() {
          cargo +"$toolchain" -Zprofile-rustflags "''${instrument[@]}" test --locked \
            --target wasm32-unknown-unknown --no-run --message-format=json-render-diagnostics "$@" >>"$output/build.log"
          cargo +"$toolchain" -Zprofile-rustflags "''${instrument[@]}" test --locked \
            --target wasm32-unknown-unknown "$@"
        }
        suite --package leptos_testing_library
        suite --package leptoskit_native --features web --test web_browser
        suite --package leptoskit_use --features local-storage --test web_storage --test web_storage_worker

        mapfile -t objects < <(
          jq -Rr 'fromjson? | select(.reason == "compiler-artifact" and .profile.test and .executable != null) | .executable' \
            "$output/build.log" | sort -u
        )
        test "''${#objects[@]}" -gt 0
        sysroot="$(rustc +"$toolchain" --print sysroot)"
        host="$(rustc +"$toolchain" -vV | sed -n 's/^host: //p')"
        llvm="$sysroot/lib/rustlib/$host/bin"
        # An unmatched glob stays literal, so a run that wrote no profile fails
        # here rather than reporting nothing.
        "$llvm/llvm-profdata" merge -sparse "$output"/*.profraw -o "$output/coverage.profdata"
        more_objects=()
        for object in "''${objects[@]:1}"; do
          more_objects+=(--object "$object")
        done
        "$llvm/llvm-cov" export --format=lcov --instr-profile "$output/coverage.profdata" \
          "''${objects[0]}" "''${more_objects[@]}" \
          --ignore-filename-regex '(\.cargo/(registry|git)/|/rustc/|\.rustup/)' |
          sed "s|^SF:$root/|SF:|" >"$output/lcov.info"
        grep -q '^SF:[^/]' "$output/lcov.info"
        echo "wrote $output/lcov.info ($(grep -c '^SF:' "$output/lcov.info") files from ''${#objects[@]} test modules)"
      '';
      description = "Measure the browser suites' coverage for the coverage gate.";
      binary = "bash";
    };
    "coverage:all" = {
      exec = ''
        set -euo pipefail
        mkdir -p target/coverage
        cargo llvm-cov clean --workspace
        cargo llvm-cov nextest --workspace --all-features --lib --tests --no-report
        # coverage_gate decides what counts; only toolchain and dependency
        # sources are dropped here, so the example apps stay in the report.
        cargo llvm-cov report --lcov --output-path target/coverage/lcov.info \
          --disable-default-ignore-filename-regex \
          --ignore-filename-regex '(\.cargo/(registry|git)/|/rustc/|\.rustup/)'
        coverage:gate "$@"
      '';
      description = "Run workspace coverage and hold it to the coverage gate (pass `--base origin/main` to gate the patch).";
      binary = "bash";
    };
    # The floor is measured by the Linux `coverage` CI job; a local run on
    # another OS compiles a different set of platform modules, so its total
    # differs.
    "coverage:gate" = {
      exec = ''
        set -euo pipefail
        cargo run --locked --quiet -p coverage_gate -- \
          --lcov target/coverage/lcov.info --floor setup/coverage_gate/floor.txt "$@"
      '';
      description = "Check target/coverage/lcov.info against the coverage floor (`--write-floor` raises it).";
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
        cd "$repo_root"
        # Addressed absolutely: the pre-commit hook runs this outside
        # `devenv shell`, where the profile bin dir is not on PATH.
        ${currentDir}/.devenv/profile/bin/leptoskit:format
        dprint fmt --config "$repo_root/dprint.json"
      '';
      description = "Format Rust with leptoskit_fmt and everything else with dprint.";
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
        run_step "lint:kotlin" lint:kotlin
        run_step "lint:releases" lint:releases
        run_step "schema:check" schema:check
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
        run_step "lint:kotlin" ${currentDir}/.devenv/profile/bin/lint:kotlin
        run_step "test:cargo" ${currentDir}/.devenv/profile/bin/test:cargo
        run_step "security:zizmor" ${currentDir}/.devenv/profile/bin/security:zizmor
        run_step "lint:releases" ${currentDir}/.devenv/profile/bin/lint:releases
        run_step "schema:check" ${currentDir}/.devenv/profile/bin/schema:check
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
    "lint:kotlin" = {
      exec = ''
        set -euo pipefail
        ktlint 'crates/**/*.kt' 'crates/**/*.kts' 'examples/**/*.kt'
      '';
      description = "Lint native Android plugin sources with ktlint.";
      binary = "bash";
    };
    "lint:swift" = {
      exec = ''
        set -euo pipefail
        swiftlint lint --strict --config .swiftlint.yml
      '';
      description = "Lint Apple plugin sources on macOS with SwiftLint.";
      binary = "bash";
    };
    "test:plugins:guest" = {
      exec = ''
        set -euo pipefail
        cargo build --locked --release --target wasm32-unknown-unknown --package plugin_guest_probe
        LEPTOSKIT_PLUGIN_GUEST="$(pwd)/target/wasm32-unknown-unknown/release/plugin_guest_probe.wasm" \
          cargo test --locked --package leptoskit_wasm --test plugins compiled_rust_guest -- --ignored
        cargo build --locked --release --target wasm32-unknown-unknown --package plugin_replies_probe
        LEPTOSKIT_PLUGIN_REPLY_GUEST="$(pwd)/target/wasm32-unknown-unknown/release/plugin_replies_probe.wasm" \
          cargo test --locked --package leptoskit_wasm --test plugin_replies -- --ignored
      '';
      description = "Run a compiled Rust plugin guest through the interpreter.";
      binary = "bash";
    };
    "test:plugins:guest-isolation" = {
      exec = ''
        set -euo pipefail
        cargo check --locked --target wasm32-unknown-unknown --no-default-features \
          --package leptoskit_bridge --package leptoskit_plugin_http --package leptoskit_plugin_websocket \
          --features leptoskit_bridge/guest,leptoskit_plugin_http/guest,leptoskit_plugin_websocket/guest
        task_guest_tree="$(cargo tree --locked --target wasm32-unknown-unknown \
          --no-default-features --edges normal --prefix none --format '{p}' \
          --package leptoskit_bridge --package leptoskit_plugin_http --package leptoskit_plugin_websocket \
          --features leptoskit_bridge/guest,leptoskit_plugin_http/guest,leptoskit_plugin_websocket/guest)"
        if grep -Eq '^(tokio|tokio-util|wasm-bindgen|js-sys|web-sys) v' <<< "$task_guest_tree"; then
          echo "Interpreter guest acquired an async host or browser dependency" >&2
          exit 1
        fi
        task_bridge_tree="$(cargo tree --locked --package leptoskit_bridge \
          --no-default-features --edges normal --prefix none --format '{p}')"
        if grep -Eq '^(tokio|tokio-util) v' <<< "$task_bridge_tree"; then
          echo "Default bridge acquired an async runtime dependency" >&2
          exit 1
        fi
        cargo test --locked --package leptoskit_bridge --no-default-features --test allocations
      '';
      description = "Check interpreter guests and the default bridge stay free of host runtimes and browser imports.";
      binary = "bash";
    };
    "test:plugins:async" = {
      exec = ''
        set -euo pipefail
        cargo test --locked --package leptoskit_bridge --package leptoskit_bridge_ffi \
          --package leptoskit_plugin_http --package leptoskit_plugin_websocket \
          --features leptoskit_bridge/async-host,leptoskit_bridge_ffi/async-host,leptoskit_plugin_http/async-host,leptoskit_plugin_websocket/async-host
      '';
      description = "Exercise optional async host jobs, scoped HTTP/WebSocket I/O and resource cleanup.";
      binary = "bash";
    };
    "test:plugins:browser" = {
      exec = ''
        set -euo pipefail
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --locked --target wasm32-unknown-unknown \
            --package leptoskit_plugin_battery \
            --package leptoskit_plugin_dialog \
            --package leptoskit_plugin_document_picker \
            --package leptoskit_plugin_screen_orientation \
            --package leptoskit_plugin_sharing \
            --package leptoskit_plugin_filesystem \
            --package leptoskit_plugin_network \
            --package leptoskit_plugin_keep_awake \
            --package leptoskit_plugin_speech \
            --package leptoskit_plugin_store \
            --features leptoskit_plugin_store/async-storage \
            --package leptoskit_plugin_http \
            --package leptoskit_plugin_logging \
            --package leptoskit_plugin_notifications \
            --package leptoskit_plugin_location \
            --package leptoskit_plugin_image_manipulator \
            --package leptoskit_sensor
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo test --locked --target wasm32-unknown-unknown \
            --package leptoskit_plugin_crypto --features web
        CHROMEDRIVER="$DEVENV_PROFILE/bin/chromedriver" \
          cargo run --locked --package leptoskit_plugin_websocket \
            --example browser_test_server -- --test
      '';
      description = "Exercise browser plugin APIs and cancellation in real Chrome.";
      binary = "bash";
    };
    "test:plugins:apple" = {
      exec = ''
        set -euo pipefail
        cargo test --locked --package leptoskit_bridge_ffi --features async-host \
          --test apple_async_lifecycle
        cargo build --locked --package leptoskit_bridge_ffi --features async-host
        rust_lib="$(pwd)/target/debug"
        swift test --package-path crates/leptoskit_bridge_ffi -Xlinker -L -Xlinker "$rust_lib"
        for manifest in crates/leptoskit_plugin_*/Package.swift crates/leptoskit_sensor/Package.swift; do
          plugin="$(dirname "$manifest")"
          if [ -d "$plugin/apple-tests" ] || [ -d "$plugin/tests/apple" ]; then
            swift test --package-path "$plugin" -Xlinker -L -Xlinker "$rust_lib"
          else
            swift build --package-path "$plugin" -Xlinker -L -Xlinker "$rust_lib"
          fi
          swift build --package-path "$plugin" \
            --triple arm64-apple-ios17.0-simulator \
            --sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)"
        done
      '';
      description = "Exercise the Rust C ABI through Swift and compile Apple plugins.";
      binary = "bash";
    };
    "test:plugins:jni" = {
      exec = ''
        set -euo pipefail
        cargo test --locked --package leptoskit_bridge_ffi --features async-host \
          --test jni_async_lifecycle -- --ignored
        cargo build --locked --package leptoskit_bridge_ffi
        mkdir -p target/plugin-jni
        task_jdk="$(dirname "$(dirname "$(realpath "$(command -v javac)")")")"
        case "$(uname -s)" in
          Darwin)
            platform=darwin
            extension=dylib
            link_flags=(-dynamiclib)
            ;;
          Linux)
            platform=linux
            extension=so
            link_flags=(-shared -ldl -lpthread -lm)
            ;;
          *) echo "JNI smoke tests require macOS or Linux" >&2; exit 1 ;;
        esac
        c++ -std=c++20 -Wall -Wextra -Werror -fPIC \
          -I "$task_jdk/include" -I "$task_jdk/include/$platform" \
          crates/leptoskit_bridge_ffi/android/bridge.cpp \
          target/debug/libleptoskit_bridge_ffi.a \
          "''${link_flags[@]}" -o "target/plugin-jni/libleptoskit_bridge_jni.$extension"
        kotlinc crates/leptoskit_bridge_ffi/android/NativePlugin.kt \
          crates/leptoskit_bridge_ffi/android-tests/BridgeSmoke.kt \
          -include-runtime -d target/plugin-jni/smoke.jar
        java -Djava.library.path=target/plugin-jni -jar target/plugin-jni/smoke.jar
      '';
      description = "Exercise real Kotlin/JNI calls, grants, and direct Rust buffer ownership.";
      binary = "bash";
    };
    "bench:bridge" = {
      exec = ''
        set -euo pipefail
        cargo run --release --package leptoskit_bridge --example roundtrip
      '';
      description = "Measure binary transport roundtrips, excluding native operations and scheduling.";
      binary = "bash";
    };
    "lint:format" = {
      exec = ''
        set -euo pipefail
        cd "$(git rev-parse --show-toplevel)"
        ${currentDir}/.devenv/profile/bin/leptoskit:format --check
        ${pkgs.dprint}/bin/dprint check
      '';
      description = "Check that all files are formatted.";
      binary = "bash";
    };
    "security:zizmor" = {
      exec = ''
        set -euo pipefail
        # --no-online-audits keeps the gate reproducible and sandbox-friendly;
        # offline audits still cover workflow injection, credential persistence,
        # dependency pinning, and permissions hardening.
        zizmor --no-online-audits --no-progress .github
      '';
      description = "Audit GitHub Actions workflows and composite actions with zizmor.";
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

    "schema:update" = {
      exec = ''
        set -euo pipefail
        cargo run --quiet --locked --package leptoskit_config --features schema --example schema
      '';
      description = "Generate public configuration schemas from Rust types.";
      binary = "bash";
    };
    "schema:check" = {
      exec = ''
        set -euo pipefail
        cargo run --quiet --locked --package leptoskit_config --features schema --example schema -- --check
      '';
      description = "Check public configuration schemas for drift.";
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
    "docs:build" = {
      exec = ''
        set -euo pipefail
        ${keepProcMacrosLoadable}
        repo_root="$(git rev-parse --show-toplevel)"
        schema:check
        cargo run --quiet --release --package leptoskit_docs_site -- build --out "$repo_root/docs/dist"
      '';
      description = "Build the documentation site into docs/dist.";
      binary = "bash";
    };
    "docs:serve" = {
      exec = ''
        set -euo pipefail
        ${keepProcMacrosLoadable}
        repo_root="$(git rev-parse --show-toplevel)"
        cargo run --quiet --release --package leptoskit_docs_site -- serve --out "$repo_root/docs/dist" "$@"
      '';
      description = "Serve the documentation site with live reload.";
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
