{
  description = "A Nix flake for working with TestChat";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;

        config.allowUnfree = true;
      };
    in
    {
      devShells."${system}".default = pkgs.mkShell {
        name = "testchat";
        packages = with pkgs; [
          protobuf
          buf

          fish
        ];
        shellHook = ''
          exec fish
        '';
      };
    };
}
