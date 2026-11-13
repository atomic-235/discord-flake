{
  lib,
  pkgs,
  ...
}:

# Built by CI, not by consumers: the resulting binary is published as the
# release asset that package.nix fetches.
let
  data = builtins.fromJSON (builtins.readFile ./version.json);
in
pkgs.pkgsStatic.rustPlatform.buildRustPackage {
  pname = "discord";
  version = data.version;

  src = lib.cleanSourceWith {
    src = ./.;
    filter = path: _: baseNameOf path != "target";
  };
  cargoLock.lockFile = ./Cargo.lock;

  doCheck = false;

  meta = with lib; {
    description = "Discord REST CLI";
    homepage = "https://github.com/atomic-235/discord-flake";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "discord";
  };
}
