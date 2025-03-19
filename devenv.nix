{ pkgs, lib, ... }:

{
  packages =
    [
      pkgs.binaryen
      pkgs.cargo-binstall
      pkgs.cargo-run-bin
      pkgs.chromedriver
      pkgs.curl
      pkgs.dprint
      pkgs.geckodriver
      pkgs.jq
      pkgs.libiconv
      pkgs.nixfmt-rfc-style
      pkgs.openssl
      pkgs.rustup
      pkgs.shfmt
    ]
    ++ lib.optionals pkgs.stdenv.isDarwin (
      with pkgs.darwin.apple_sdk;
      [
        pkgs.coreutils
        frameworks.CoreFoundation
        frameworks.Security
        frameworks.System
        frameworks.SystemConfiguration
      ]
    );

  # disable dotenv since it breaks the variable interpolation supported by `direnv`
  dotenv.disableHint = true;

  tasks = {
    "rustfmt:nightly" = {
      exec = "rustup toolchain install nightly --component rustfmt --force";
      before = [ "devenv:enterShell" ];
    };
  };

  scripts.anchor = {
    exec = ''
      set -e
      cargo bin anchor $@
    '';
    description = "The `anchor` executable";
		binary = "bash";
  };
  scripts."release-plz" = {
    exec = ''
      set -e
      cargo bin release-plz $@
    '';
    description = "The `release-plz` executable";
		binary = "bash";
  };
  scripts."wasm-bindgen" = {
    exec = ''
      set -e
      cargo bin wasm-bindgen $@
    '';
    description = "The `wasm-bindgen` executable";
		binary = "bash";
  };
  scripts."wasm-bindgen-test-runner" = {
    exec = ''
      set -e
      cargo bin wasm-bindgen-test-runner $@
    '';
    description = "The `wasm-bindgen-test-runner` executable";
		binary = "bash";
  };
  scripts."install:all" = {
    exec = ''
      set -e
      install:cargo:bin
    '';
    description = "Install all packages.";
		binary = "bash";
  };
  scripts."install:cargo:bin" = {
    exec = ''
      set -e
      cargo bin --install
    '';
    description = "Install cargo binaries locally.";
		binary = "bash";
  };
  scripts."update:deps" = {
    exec = ''
      set -e
      cargo update
      devenv update
    '';
    description = "Update dependencies.";
		binary = "bash";
  };
  scripts."build:all" = {
    exec = ''
      set -e
      if [ -z "$CI" ]; then
        echo "Builing project locally"
        cargo build --all-features
      else
        echo "Building in CI"
        cargo build --all-features --locked
      fi
    '';
    description = "Build all crates with all features activated.";
		binary = "bash";
  };
  scripts."test:all" = {
    exec = ''
      set -e
      cargo test_leptoskit
    '';
    description = "Run all tests across the crates";
		binary = "bash";
  };

  scripts."fix:all" = {
    exec = ''
      set -e
      fix:clippy
      fix:format
    '';
    description = "Fix all autofixable problems.";
		binary = "bash";
  };
  scripts."fix:format" = {
    exec = ''
      set -e
      dprint fmt --config "$DEVENV_ROOT/dprint.json"
    '';
    description = "Format files with dprint.";
		binary = "bash";
  };
  scripts."fix:clippy" = {
    exec = ''
      set -e
      cargo clippy --fix --allow-dirty --allow-staged --all-features
    '';
    description = "Fix clippy lints for rust.";
		binary = "bash";
  };
  scripts."lint:all" = {
    exec = ''
      set -e
      lint:clippy
      lint:format
    '';
    description = "Run all checks.";
		binary = "bash";
  };
  scripts."lint:format" = {
    exec = ''
      set -e
      dprint check
    '';
    description = "Check that all files are formatted.";
		binary = "bash";
  };
  scripts."lint:clippy" = {
    exec = ''
      set -e
      cargo clippy --all-features
    '';
    description = "Check that all rust lints are passing.";
		binary = "bash";
  };
  scripts."setup:vscode" = {
    exec = ''
      set -e
      rm -rf .vscode
      cp -r $DEVENV_ROOT/setup/editors/vscode .vscode
    '';
    description = "Setup the environment for vscode.";
		binary = "bash";
  };
  scripts."setup:helix" = {
    exec = ''
      set -e
      rm -rf .helix
      cp -r $DEVENV_ROOT/setup/editors/helix .helix
    '';
    description = "Setup for the helix editor.";
		binary = "bash";
  };
  scripts."setup:ci" = {
    exec = ''
      set -e
      # update github ci path
      echo "$DEVENV_PROFILE/bin" >> $GITHUB_PATH
      echo "$GITHUB_WORKSPACE/.local-cache/solana-release/bin" >> $GITHUB_PATH

      # update github ci environment
      echo "DEVENV_PROFILE=$DEVENV_PROFILE" >> $GITHUB_ENV

      # prepend common compilation lookup paths
      echo "PKG_CONFIG_PATH=$PKG_CONFIG_PATH" >> $GITHUB_ENV
      echo "LD_LIBRARY_PATH=$LD_LIBRARY_PATH" >> $GITHUB_ENV
      echo "LIBRARY_PATH=$LIBRARY_PATH" >> $GITHUB_ENV
      echo "C_INCLUDE_PATH=$C_INCLUDE_PATH" >> $GITHUB_ENV

      # these provide shell completions / default config options
      echo "XDG_DATA_DIRS=$XDG_DATA_DIRS" >> $GITHUB_ENV
      echo "XDG_CONFIG_DIRS=$XDG_CONFIG_DIRS" >> $GITHUB_ENV

      echo "DEVENV_DOTFILE=$DEVENV_DOTFILE" >> $GITHUB_ENV
      echo "DEVENV_PROFILE=$DEVENV_PROFILE" >> $GITHUB_ENV
      echo "DEVENV_ROOT=$DEVENV_ROOT" >> $GITHUB_ENV
      echo "DEVENV_STATE=$DEVENV_STATE" >> $GITHUB_ENV

      # Temporary fix for a bug in anchor@0.30.1 https://github.com/coral-xyz/anchor/issues/3042
      echo "ANCHOR_IDL_BUILD_PROGRAM_PATH=$DEVENV_ROOT/programs/example_program" >> $GITHUB_ENV
    '';
    description = "Setup devenv for GitHub Actions";
		binary = "bash";
  };
  scripts."setup:docker" = {
    exec = ''
      set -e
      # update path
      echo "export PATH=$DEVENV_PROFILE/bin:\$PATH" >> /etc/profile
      echo "export PATH=$DEVENV_ROOT/.local-cache/solana-release/bin:\$PATH" >> /etc/profile

      echo "export DEVENV_PROFILE=$DEVENV_PROFILE" >> /etc/profile
      echo "export PKG_CONFIG_PATH=$PKG_CONFIG_PATH" >> /etc/profile
      echo "export LD_LIBRARY_PATH=$LD_LIBRARY_PATH" >> /etc/profile
      echo "export LIBRARY_PATH=$LIBRARY_PATH" >> /etc/profile
      echo "export C_INCLUDE_PATH=$C_INCLUDE_PATH" >> /etc/profile
      echo "export XDG_DATA_DIRS=$XDG_DATA_DIRS" >> /etc/profile
      echo "export XDG_CONFIG_DIRS=$XDG_CONFIG_DIRS" >> /etc/profile

      echo "export DEVENV_DOTFILE=$DEVENV_DOTFILE" >> /etc/profile
      echo "export DEVENV_PROFILE=$DEVENV_PROFILE" >> /etc/profile
      echo "export DEVENV_ROOT=$DEVENV_ROOT" >> /etc/profile
      echo "export DEVENV_STATE=$DEVENV_STATE" >> /etc/profile

      # Temporary fix for a bug in anchor@0.30.1 https://github.com/coral-xyz/anchor/issues/3042
      echo "export ANCHOR_IDL_BUILD_PROGRAM_PATH=$DEVENV_ROOT/programs/example_program" >> /etc/profile
    '';
    description = "Setup devenv shell for docker.";
		binary = "bash";
  };
}
