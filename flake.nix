{
  description = "codj - vim-style terminal music player and keyboard DJ";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    utils.url = "github:numtide/flake-utils";
    codjd = {
      url = "github:hitchho/codjd";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.utils.follows = "utils";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      utils,
      codjd,
    }:
    let
      codjFor =
        pkgs: codjdPkg:
        (codjd.lib.rustPlatformFor pkgs).buildRustPackage {
          pname = "codj";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.makeWrapper
          ];
          buildInputs = [ pkgs.luajit ];
          # tests want a session bus and a running codjd
          doCheck = false;
          # codj starts codjd (found in PATH) with setsid when it is not running
          postInstall = ''
            wrapProgram $out/bin/codj \
              --prefix PATH : ${
                pkgs.lib.makeBinPath [
                  codjdPkg
                  pkgs.util-linux
                ]
              }
          '';
          meta = {
            description = "codj - vim-style terminal music player and keyboard DJ";
            homepage = "https://github.com/hitchho/codj";
            license = pkgs.lib.licenses.gpl3Plus;
            mainProgram = "codj";
            platforms = pkgs.lib.platforms.linux;
          };
        };
    in
    {
      overlays.default =
        final: prev:
        let
          codjdPkg = (codjd.overlays.default final prev).codjd;
        in
        {
          codjd = codjdPkg;
          codj = codjFor final codjdPkg;
        };
    }
    // utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        codjdPkg = codjd.packages.${system}.codjd;
        codj = codjFor pkgs codjdPkg;
      in
      {
        packages = {
          inherit codj;
          codjd = codjdPkg;
          default = codj;
        };
        apps.default = utils.lib.mkApp { drv = codj; };
        devShells.default = pkgs.mkShell {
          inputsFrom = [ codj ];
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
            pkgs.rust-analyzer
          ];
        };
      }
    );
}
