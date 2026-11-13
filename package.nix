{
  lib,
  stdenvNoCC,
  fetchurl,
  makeWrapper,
}:

let
  data = builtins.fromJSON (builtins.readFile ./version.json);
  system = stdenvNoCC.hostPlatform.system;

  asset =
    {
      x86_64-linux = "discord-linux-x86_64";
    }
    .${system} or (throw "discord: unsupported system ${system}");
in
stdenvNoCC.mkDerivation {
  pname = "discord";
  version = data.version;

  # Static musl binary with zero /nix/store references, so a plain
  # fixed-output fetch is enough.
  src = fetchurl {
    url = "https://github.com/atomic-235/discord-flake/releases/download/v${data.version}/${asset}";
    hash = data.assets.${system};
  };

  dontUnpack = true;
  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    install -Dm755 "$src" "$out/bin/discord"
    runHook postInstall
  '';

  meta = with lib; {
    description = "Discord REST CLI (static release binary)";
    homepage = "https://github.com/atomic-235/discord-flake";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "discord";
  };
}
