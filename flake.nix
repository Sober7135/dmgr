{
  description = "dmgr";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      for_all_systems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = for_all_systems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "dmgr";
            version = "0.1.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;

            meta = with pkgs.lib; {
              description = "Local-first Docker development manager";
              mainProgram = "dmgr";
              license = licenses.asl20;
              platforms = platforms.unix;
            };
          };
        }
      );

      apps = for_all_systems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/dmgr";
        };
      });
    };
}
