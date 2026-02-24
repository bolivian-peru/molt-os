# osModa — AI-Native Operating System
# NixOS + OpenClaw = Agents as first-class OS citizens
{
  description = "osModa: AI-native operating system powered by OpenClaw on NixOS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    # Rust builds via crane
    crane.url = "github:ipetkov/crane";

    # OpenClaw Nix packaging
    nix-openclaw = {
      url = "github:openclaw/nix-openclaw";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Image builders (ISO, qcow2, raw, etc.)
    nixos-generators = {
      url = "github:nix-community/nixos-generators";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Declarative disk partitioning
    disko = {
      url = "github:nix-community/disko";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Encrypted secrets management
    agenix = {
      url = "github:ryantm/agenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Home manager for user-level config
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, crane, nix-openclaw, nixos-generators, disko, agenix, home-manager }:
  let
    # Shared NixOS modules used by all configurations
    sharedModules = [
      ./nix/modules/osmoda.nix
      ./nix/modules/osmoda-shell.nix
      ./nix/modules/osmoda-setup.nix
      ./nix/modules/osmoda-ui.nix
      agenix.nixosModules.default
      {
        nixpkgs.overlays = [
          nix-openclaw.overlays.default
          self.overlays.default
        ];
      }
    ];

    # x86_64 pkgs + crane (for top-level packages and checks)
    linuxSystem = "x86_64-linux";
    linuxPkgs = import nixpkgs {
      system = linuxSystem;
      config.allowUnfree = true;
      overlays = [
        nix-openclaw.overlays.default
        self.overlays.default
      ];
    };

    craneLibLinux = crane.mkLib linuxPkgs;

    # Shared Rust build args (used by checks only — overlay has its own)
    checkArgs = {
      src = craneLibLinux.cleanCargoSource ./.;
      strictDeps = true;
      buildInputs = with linuxPkgs; [ sqlite openssl ];
      nativeBuildInputs = with linuxPkgs; [ pkg-config ];
    };

    cargoArtifacts = craneLibLinux.buildDepsOnly checkArgs;
  in
  {
    # --- Overlays ---
    # Architecture-aware: builds Rust crates for whatever system it's applied to
    overlays.default = final: prev: let
      craneLib = crane.mkLib final;
      src = craneLib.cleanCargoSource ./.;
      commonArgs = {
        inherit src;
        strictDeps = true;
        buildInputs = with final; [ sqlite openssl ];
        nativeBuildInputs = with final; [ pkg-config ];
      };
      artifacts = craneLib.buildDepsOnly commonArgs;
    in {
      osmoda-agentd = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p agentd";
      });
      osmoda-agentctl = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p agentctl";
      });
      osmoda-egress = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-egress";
      });
      osmoda-voice = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-voice";
      });
      osmoda-keyd = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-keyd";
      });
      osmoda-watch = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-watch";
      });
      osmoda-routines = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-routines";
      });
      osmoda-mesh = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-mesh";
      });
      osmoda-mcpd = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-mcpd";
      });
      osmoda-teachd = craneLib.buildPackage (commonArgs // {
        cargoArtifacts = artifacts;
        cargoExtraArgs = "-p osmoda-teachd";
      });
      osmoda-system-skills = final.callPackage ./packages/osmoda-system-skills { };
      osmoda-plymouth-theme = final.callPackage ./nix/modules/plymouth-theme { };
    };

    # --- NixOS Configurations ---
    nixosConfigurations = {
      # === x86_64 ===

      # Dev VM with Sway desktop + OpenClaw + agentd
      osmoda-dev = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/dev-vm.nix
          home-manager.nixosModules.home-manager
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
          }
        ];
      };

      # Headless server
      osmoda-server = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/server.nix
        ];
      };

      # Hetzner VPS (deploy via nixos-anywhere)
      osmoda-hetzner = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/hetzner.nix
          disko.nixosModules.disko
        ];
      };

      # Installer ISO (x86_64)
      osmoda-iso = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/iso.nix
          "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
        ];
      };

      # === aarch64 (ARM — for M2 Mac via UTM, ARM servers) ===

      # Dev VM (aarch64)
      osmoda-dev-arm = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/dev-vm.nix
          home-manager.nixosModules.home-manager
          {
            home-manager.useGlobalPkgs = true;
            home-manager.useUserPackages = true;
          }
        ];
      };

      # Installer ISO (aarch64)
      osmoda-iso-arm = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        modules = sharedModules ++ [
          ./nix/hosts/iso.nix
          "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
        ];
      };
    };

    # --- Packages (x86_64) ---
    packages.${linuxSystem} = {
      agentd = linuxPkgs.osmoda-agentd;
      agentctl = linuxPkgs.osmoda-agentctl;
      egress = linuxPkgs.osmoda-egress;
      voice = linuxPkgs.osmoda-voice;
      keyd = linuxPkgs.osmoda-keyd;
      watch = linuxPkgs.osmoda-watch;
      routines = linuxPkgs.osmoda-routines;
      mesh = linuxPkgs.osmoda-mesh;
      mcpd = linuxPkgs.osmoda-mcpd;
      teachd = linuxPkgs.osmoda-teachd;
      default = linuxPkgs.osmoda-agentd;
    };

    # --- Checks (x86_64) ---
    checks.${linuxSystem} = {
      osmoda-clippy = craneLibLinux.cargoClippy (checkArgs // {
        inherit cargoArtifacts;
        cargoClippyExtraArgs = "--all-targets -- --deny warnings";
      });
      osmoda-tests = craneLibLinux.cargoTest (checkArgs // {
        inherit cargoArtifacts;
      });
    };
  } //
  # Multi-system devShells (macOS dev + Linux target)
  flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" "x86_64-darwin" ] (system:
    let
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
      };
    in {
      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          # Rust
          rustc
          cargo
          rust-analyzer
          clippy
          rustfmt
          # Build deps
          pkg-config
          sqlite
          openssl
          # Node.js (for osmoda-bridge plugin)
          nodejs_22
          pnpm
          # Nix tools
          jq
          nixos-rebuild
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.Security
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ];
        shellHook = ''
          echo "osModa development shell"
          echo "  cargo check --workspace         - Check Rust code"
          echo "  cargo test --workspace          - Run tests"
          echo "  cargo run -p agentd -- --help   - Run agentd"
        '';
      };
    }
  );
}
