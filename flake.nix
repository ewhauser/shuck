{
  description = "shuck dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.hyperfine
            pkgs.samply
            pkgs.python3
            pkgs.shellcheck
            pkgs.shfmt
            pkgs.cargo-fuzz
            pkgs.cargo-udeps
            pkgs.yq
          ];
        };
      }
    );
}
