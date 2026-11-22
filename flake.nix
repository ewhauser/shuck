{
  description = "A fast shell script linter";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    rust-overlay,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {inherit system overlays;};

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        # Packages used in testing environment.
        testPackages = with pkgs; [
          hyperfine
          samply
          neovim
          python3
          lua
          shellcheck
          shfmt
          zsh
          cargo-fuzz
          cargo-shear
          yq
        ];
      in {
        devShells.default = pkgs.mkShell {
          packages = testPackages;
        };

        packages = rec {
          shuck = pkgs.callPackage ./nix/package.nix {
            inherit rustPlatform;
            nativeBuildInputs = testPackages;
          };

          default = shuck;
        };
      }
    );
}
