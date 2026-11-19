{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: let
  # Native libraries the UI stack loads at runtime (winit/wayland-sys, wgpu,
  # font-kit) plus the ones GPUI links. Both window systems are included so
  # the app runs on Wayland or X11. Paths are embedded in the binary's rpath
  # via languages.rust.rustflags.
  dlopenLibraries = with pkgs; [
    # GPU backend
    vulkan-loader
    libGL
    # Window system
    wayland
    libxkbcommon
    libx11
    libxcb
    libxcursor
    libxi
    libxrandr
    # GPUI/zed-font-kit discovers fonts through fontconfig/freetype
    fontconfig
    freetype
    # rfd's xdg-desktop-portal backend dlopens libdbus-1.so
    dbus
  ];
in {
  # https://devenv.sh/basics/
  env.GREET = "devenv";

  # https://devenv.sh/packages/
  packages =
    [
      pkgs.git
      # pkg-config finds ALSA (cpal, via rodio — see sound.md)
      pkgs.pkg-config
      pkgs.alsa-lib
      # fallback folder picker when the portal is unreachable
      pkgs.zenity
    ]
    ++ dlopenLibraries;

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
    # https://devenv.sh/reference/options/#languagesrustchannel
    channel = "stable";
    version = "1.100.0";
    # Embed the dlopen library paths in the binary's rpath so `cargo run`
    # works directly inside the shell on NixOS.
    rustflags = "-C link-arg=-Wl,-rpath,${lib.makeLibraryPath dlopenLibraries}";
  };

  # https://devenv.sh/processes/
  # processes.dev.exec = "${lib.getExe pkgs.watchexec} -n -- ls -la";

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  # https://devenv.sh/scripts/
  scripts.hello.exec = ''
    echo hello from $GREET
  '';

  # https://devenv.sh/basics/
  enterShell = ''
  '';

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  enterTest = ''
  '';

  # https://devenv.sh/git-hooks/
  # git-hooks.hooks.shellcheck.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
