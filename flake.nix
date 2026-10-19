{
  description = "coldsonic — a music player for Subsonic servers";

  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    devenv.url = "github:cachix/devenv";
    crane.url = "github:ipetkov/crane";
  };

  nixConfig = {
    extra-trusted-public-keys = "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw=";
    extra-substituters = "https://devenv.cachix.org";
  };

  outputs = {
    self,
    nixpkgs,
    devenv,
    crane,
    ...
  } @ inputs: let
    systems = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    forAllSystems = nixpkgs.lib.genAttrs systems;
    cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      craneLib = crane.mkLib pkgs;
      dlopenLibraries = with pkgs; [
        vulkan-loader
        libGL
        wayland
        libxkbcommon
        libx11
        libxcursor
        libxi
        libxrandr
        # rfd's xdg-desktop-portal backend dlopens libdbus-1.so
        dbus
      ];
      commonArgs = {
        pname = cargoToml.package.name;
        version = cargoToml.package.version;
        src = craneLib.cleanCargoSource ./.;
        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.makeWrapper
        ];
        buildInputs =
          [pkgs.alsa-lib]
          ++ dlopenLibraries;
        env.RUSTFLAGS = "-C link-arg=-Wl,-rpath,${pkgs.lib.makeLibraryPath dlopenLibraries}";
        dontPatchELF = true;
        # zenity is rfd's fallback folder picker; put it on the app's PATH
        postInstall = ''
          wrapProgram $out/bin/coldsonic --prefix PATH : ${pkgs.lib.makeBinPath [pkgs.zenity]}
        '';
      };
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
    in {
      default = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;

          meta = {
            mainProgram = cargoToml.package.name;
          };
        });
    });

    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      default = devenv.lib.mkShell {
        inherit inputs pkgs;
        modules = [./devenv.nix];
      };
    });
  };
}
