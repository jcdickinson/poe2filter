{
  description = "POE2 Item Filter Auto Updater";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      supportedSystems = [ "x86_64-linux" ];
      overlays = [ (import rust-overlay) ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      nixpkgsFor = forAllSystems (system: import nixpkgs { inherit system overlays; });
      toml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system}.pkgsStatic;
        in rec {
          poe2filter = pkgs.rustPlatform.buildRustPackage {
            pname = "poe2filter";
            version = toml.package.version;

            src = ./.;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
          };
          default = poe2filter;
        });

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgsFor.${system};
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer" "cargo" "clippy" "rustfmt" ];
          };
        in {
          default = pkgs.mkShell {
            buildInputs = [
              rustToolchain
            ];
          };
        });
    };
}
