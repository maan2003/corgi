{ inputs, ... }:
pkgs:
let
  rustBin = inputs.rust-overlay.lib.mkRustBin { } pkgs;
  rustToolchain = (rustBin.fromRustupToolchainFile ../rust-toolchain.toml).override {
    extensions = [
      "clippy"
      "rustfmt"
    ];
    # The supplied-target-tools test builds for a target that Corgi's Zig does not support.
    targets = [ "x86_64-linux-android" ];
  };
in
pkgs.callPackage ./package.nix {
  crane = (inputs.crane.mkLib pkgs).overrideScope (
    _: _: {
      stdenvSelector = packages: packages.clangStdenv;
    }
  );
  inherit rustToolchain;
}
