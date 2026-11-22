{
  lib,
  makeWrapper,
  nativeBuildInputs,
  nix,
  rustPlatform,
}: let
  workspaceTOML = (fromTOML (builtins.readFile ../Cargo.toml)).workspace.package;
  packageTOML = (fromTOML (builtins.readFile ../crates/shuck-cli/Cargo.toml)).package;
in
  rustPlatform.buildRustPackage {
    inherit (workspaceTOML) version;

    pname = "shuck";

    src = ../.;

    cargoLock.lockFile = ../Cargo.lock;
    cargoBuildFlags = ["-p" packageTOML.name];

    checkPhase = ''
      runHook preCheck
      make check-nix
      runHook postCheck
    '';

    nativeBuildInputs =
      [
        makeWrapper
      ]
      ++ (lib.optionals (nativeBuildInputs != null) nativeBuildInputs);

    # Nix's hardened GCC defines `_FORTIFY_SOURCE` unconditionally, which emits a
    # warning when optimization is disabled. jemalloc's `configure` compiles a test
    # with `-O0` plus `-Werror`, so that warning fails the build.
    hardeningDisable = ["fortify"];

    meta = {
      inherit (packageTOML) description;

      homepage = "https://github.com/ewhauser/shuck";
      license = lib.licenses.mit;
      platforms = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-windows"
        "x86_64-linux"
      ];
    };
  }
