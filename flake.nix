{
  description = "Development shell for tmc-langs-rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        lib = pkgs.lib;

        # Pin the toolchain. Kept here rather than in a rust-toolchain.toml on
        # purpose: a checked-in rust-toolchain.toml would force-pin rustup for
        # every non-nix contributor too, which we don't want.
        #
        # Note: the workspace Cargo.toml still declares rust-version = "1.85.0",
        # but that MSRV is stale — the current dependency graph cannot be built
        # with it. In particular `exercise-services-api` (pulled in via the
        # active local [patch] to ../secret-project-331) requires rustc 1.96.0,
        # and several transitive deps (time, zip, icu_*, cookie_store) require
        # 1.86–1.88. 1.96 matches secret-project-331's own rust-toolchain.toml
        # pin, so we pin the same here to actually build/test the tree.
        rustToolchain = pkgs.rust-bin.stable."1.96.0".default;

        # The R test runner used by the R plugin is not in nixpkgs, so build it
        # from the copy vendored in this repo (no network fetch). Its runtime
        # deps come from its DESCRIPTION: Depends testthat; Imports jsonlite,
        # R.utils.
        tmcRtestrunner = pkgs.rPackages.buildRPackage {
          name = "tmcRtestrunner";
          src = ./crates/plugins/r/tests/tmcRtestrunner;
          propagatedBuildInputs = with pkgs.rPackages; [
            testthat
            jsonlite
            R_utils
          ];
        };

        # R environment that has the test runner (and its deps) on the library
        # path, so `Rscript -e 'library(tmcRtestrunner)'` works out of the box.
        rEnv = pkgs.rWrapper.override {
          packages = [
            tmcRtestrunner
            pkgs.rPackages.testthat
            pkgs.rPackages.jsonlite
            pkgs.rPackages.R_utils
          ];
        };

        # `check` ships check.pc in $out/lib/pkgconfig (single output); openssl
        # and zlib expose theirs from their .dev outputs.
        pkgConfigPath = lib.makeSearchPath "lib/pkgconfig" [
          pkgs.openssl.dev
          pkgs.zlib.dev
          pkgs.check
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            # Rust
            rustToolchain

            # Native build tooling (bindgen -> clang-sys is in the tree, hence libclang)
            pkgs.pkg-config
            pkgs.gcc
            pkgs.gnumake
            pkgs.cmake
            pkgs.openssl
            pkgs.zlib
            pkgs.zstd
            pkgs.libclang

            # Java plugin (Maven + Ant) and the embedded JVM via j4rs
            pkgs.jdk21
            pkgs.maven
            pkgs.ant

            # C# plugin (samples target net8.0)
            pkgs.dotnet-sdk_8

            # Make plugin: `check` unit-test framework + valgrind (gnumake above)
            pkgs.check
            pkgs.valgrind

            # Python plugin
            pkgs.python3

            # Node: TS bindings generation (scripts/generate-*-ts-bindings)
            pkgs.nodejs

            # R plugin (bundles the tmcRtestrunner runner + its deps)
            rEnv
          ];

          # bindgen (clang-sys) needs to find libclang at build time.
          LIBCLANG_PATH = lib.makeLibraryPath [ pkgs.libclang.lib ];

          # Point the embedded JVM (j4rs) at the nix-provided JDK, so it
          # resolves libzip/libz against the nix loader.
          JAVA_HOME = pkgs.jdk21.home;

          shellHook = ''
            export PATH="${pkgs.jdk21.home}/bin:$PATH"
            export PKG_CONFIG_PATH="${pkgConfigPath}''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
          '';
        };
      }
    );
}
