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

        # Pinned here, not in a rust-toolchain.toml, which would also pin rustup for
        # non-nix contributors. Keep equal to the workspace Cargo.toml's rust-version
        # (floored by `exercise-services-api`) and secret-project-331's toolchain pin.
        rustToolchain = pkgs.rust-bin.stable."1.96.0".default;

        # Not in nixpkgs; built from the vendored copy. Deps mirror its DESCRIPTION.
        tmcRtestrunner = pkgs.rPackages.buildRPackage {
          name = "tmcRtestrunner";
          src = ./crates/plugins/r/tests/tmcRtestrunner;
          propagatedBuildInputs = with pkgs.rPackages; [
            testthat
            jsonlite
            R_utils
          ];
        };

        rEnv = pkgs.rWrapper.override {
          packages = [
            tmcRtestrunner
            pkgs.rPackages.testthat
            pkgs.rPackages.jsonlite
            pkgs.rPackages.R_utils
          ];
        };

        # `check` ships check.pc in its sole output; openssl and zlib in .dev.
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

            # Native build tooling (bindgen needs libclang)
            pkgs.pkg-config
            pkgs.gcc
            pkgs.gnumake
            pkgs.cmake
            pkgs.openssl
            pkgs.zlib
            pkgs.zstd
            pkgs.libclang

            # Java plugin and j4rs's embedded JVM
            pkgs.jdk21
            pkgs.maven
            pkgs.ant

            # C# plugin (samples target net8.0)
            pkgs.dotnet-sdk_8

            # Make plugin (gnumake above)
            pkgs.check
            pkgs.valgrind

            # Python plugin
            pkgs.python3

            # Node: TS bindings generation
            pkgs.nodejs

            # R plugin
            rEnv
          ];

          # bindgen needs libclang at build time.
          LIBCLANG_PATH = lib.makeLibraryPath [ pkgs.libclang.lib ];

          # Makes j4rs's embedded JVM resolve libzip/libz against the nix loader.
          JAVA_HOME = pkgs.jdk21.home;

          shellHook = ''
            export PATH="${pkgs.jdk21.home}/bin:$PATH"
            export PKG_CONFIG_PATH="${pkgConfigPath}''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
          '';
        };
      }
    );
}
