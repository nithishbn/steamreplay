{
  description = "A basic flake with a shell";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    systems.url = "github:nix-systems/default";
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
    };
  };
  outputs = {
    nixpkgs,
    flake-utils,

    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = (import nixpkgs) {
          inherit system;
        };
      in let
        myR = pkgs.rWrapper.override {
          packages = with pkgs.rPackages; [
            tidyverse
            ggplot2
            rmarkdown
          ];
        };
      in {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            bashInteractive
            cargo
            rustc
            rust-analyzer
            myR
            quarto
            (rstudioWrapper.override {
              packages = with rPackages; [
                tidyverse
                ggplot2
                rmarkdown
              ];
            })
          ];

        };
      }
    );
}
