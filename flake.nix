{
  description = "Fedi stability pool Fedimint module";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-utils.url = "github:numtide/flake-utils";
    flakebox = {
      url = "github:dpc/flakebox?rev=52e287e576e92644d04d43def75307f85b226a52";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    selfci = {
      url = "github:dpc/selfci";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      flakebox,
      selfci,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        projectName = "fedi-stability-pool";

        selfciPkg = selfci.packages.${system}.default;
        selfciCheck = pkgs.writeShellApplication {
          name = "selfci-check";
          runtimeInputs = [
            selfciPkg
            pkgs.git
          ];
          text = ''
            exec selfci check "$@"
          '';
        };
        mq = pkgs.writeShellScriptBin "mq" ''
          exec ${selfciPkg}/bin/selfci mq "$@"
        '';

        flakeboxLib = flakebox.lib.mkLib pkgs {
          config = {
            github.ci.buildOutputs = [ ".#ci.workspace" ];
            just.rules.watch.enable = false;
            rootDir.".envrc".text = pkgs.lib.mkAfter ''
              source_env_if_exists .envrc.local
            '';
            # The stock hook also checks .config/flakebox/shellHook.sh, which
            # flakebox generates with an unused variable (SC2034) at this pin.
            # Skip the generated files; check every other shell script.
            git.pre-commit.hooks.shellcheck = pkgs.lib.mkForce ''
              shellcheck_paths=()
              while IFS= read -r path; do
                if [[ "$path" == *.sh ]] && [[ "$path" != .config/flakebox/* ]]; then
                  shellcheck_paths+=("$path")
                fi
              done <<< "$FLAKEBOX_GIT_LS_TEXT"

              if [[ "''${#shellcheck_paths[@]}" -ne 0 ]]; then
                shellcheck --severity=warning "''${shellcheck_paths[@]}"
              fi
            '';
            toolchain.components = [
              "rustc"
              "cargo"
              "clippy"
              "rust-analyzer"
              "rust-src"
              "rustfmt"
            ];
          };
        };

        buildSrc = flakeboxLib.source.fromPaths {
          root = ./.;
          paths = [
            "Cargo.toml"
            "Cargo.lock"
            "crates"
          ];
        };

        multiBuild = (flakeboxLib.craneMultiBuild { }) (
          craneLib':
          let
            craneLib = craneLib'.overrideArgs {
              pname = projectName;
              src = buildSrc;
              # aws-lc-sys (via the fedimint client stack) builds with cmake;
              # some crates also discover native libs through pkg-config.
              nativeBuildInputs = [
                pkgs.cmake
                pkgs.pkg-config
              ];
              dontUseCmakeConfigure = true;
              env.RUSTDOCFLAGS = "-D warnings";
            };
          in
          rec {
            workspaceDeps = craneLib.buildWorkspaceDepsOnly { };

            workspace = craneLib.buildWorkspace {
              cargoArtifacts = workspaceDeps;
            };

            clippy = craneLib.cargoClippy {
              cargoArtifacts = workspaceDeps;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              doInstallCargoArtifacts = false;
            };

            tests = craneLib.cargoTest {
              cargoArtifacts = workspace;
              doInstallCargoArtifacts = false;
            };

            cargoFmt = craneLib.cargoFmt { };
          }
        );

        treefmt =
          pkgs.runCommand "treefmt-check"
            {
              nativeBuildInputs = [
                pkgs.treefmt
                pkgs.nixfmt-rfc-style
                pkgs.rustfmt
                pkgs.taplo
              ];
              src = self;
            }
            ''
              cp -r $src work && chmod -R u+w work
              cd work
              treefmt --ci
              touch $out
            '';
      in
      {
        packages = {
          inherit treefmt;
          default = multiBuild.workspace;
        };

        apps = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          selfci-check = {
            type = "app";
            program = "${selfciCheck}/bin/selfci-check";
          };
        };

        ci = {
          inherit (multiBuild.ci)
            workspace
            clippy
            tests
            cargoFmt
            ;
        };

        legacyPackages = multiBuild;

        devShells = flakeboxLib.mkShells {
          packages = [
            pkgs.pkg-config
            pkgs.cargo-nextest
            # aws-lc-sys (via fedimint) builds with cmake
            pkgs.cmake
            pkgs.taplo
          ]
          # selfci (and the `mq` wrapper) call the Linux-only pidfd syscall and
          # fail to build on darwin, which blocks the whole dev shell. They are a
          # dev convenience, not a build dependency, so gate them to Linux.
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            selfciPkg
            mq
          ];
        };
      }
    );

  nixConfig = {
    extra-substituters = [ "https://fedimint.cachix.org" ];
    extra-trusted-public-keys = [
      "fedimint.cachix.org-1:FpJJjy1iPVlvyv4OMiN5y9+/arFLPcnZhZVVCHCDYTs="
    ];
  };
}
