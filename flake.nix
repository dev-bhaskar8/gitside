{
  description = "Gitside — source control that fits your terminal";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/1559d3daa3ecc813a650b79375ea61b6741b8746";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in {
      packages = forAllSystems (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "gitside";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.dbus ];
            nativeCheckInputs = [ pkgs.git pkgs.python3 ];
            doCheck = true;
            # The tests create temporary CLI fixtures; these shebangs must also
            # resolve inside Nix's filesystem sandbox, not only on FHS hosts.
            postPatch = ''
              substituteInPlace src/ai.rs src/ai/provider/acp.rs \
                --replace-fail '#!/bin/sh' '#!${pkgs.runtimeShell}'
              substituteInPlace src/ai/provider/acp.rs src/ai/provider/rpc.rs \
                --replace-fail '#!/usr/bin/env python3' '#!${pkgs.python3}/bin/python3'
            '';
            postInstall = ''
              install -Dm644 LICENSE $out/share/licenses/gitside/LICENSE
              install -Dm644 docs/third-party-notices.md $out/share/licenses/gitside/third-party-notices.md
            '';
            meta = with pkgs.lib; {
              description = "VS Code-inspired source control panel for the terminal";
              homepage = "https://dev-bhaskar8.github.io/gitside/";
              license = licenses.mit;
              mainProgram = "gitside";
              platforms = platforms.unix;
            };
          };
        });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/gitside";
        };
      });
    };
}
