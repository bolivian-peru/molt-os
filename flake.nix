# AgentOS (molt-os) — AI-Native Operating System
# NixOS + OpenClaw = Agents as first-class OS citizens
{
  description = "AgentOS: AI-native operating system powered by OpenClaw on NixOS";

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
    # NixOS configurations (Linux only)
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

    # Rust crate builds
    commonArgs = {
      src = craneLibLinux.cleanCargoSource ./.;
      strictDeps = true;
      buildInputs = with linuxPkgs; [ sqlite openssl ] ++ linuxPkgs.lib.optionals linuxPkgs.stdenv.isDarwin [
        linuxPkgs.darwin.apple_sdk.frameworks.Security
        linuxPkgs.darwin.apple_sdk.frameworks.SystemConfiguration
      ];
      nativeBuildInputs = with linuxPkgs; [ pkg-config ];
    };

    cargoArtifacts = craneLibLinux.buildDepsOnly commonArgs;

    # Shared NixOS modules used by all configurations
    sharedModules = [
      ./nix/modules/agentos.nix
      ./nix/modules/agentos-shell.nix
      ./nix/modules/agentos-setup.nix
      agenix.nixosModules.default
      {
        nixpkgs.overlays = [
          nix-openclaw.overlays.default
          self.overlays.default
        ];
      }
    ];
  in
  {
    # --- Overlays ---
    overlays.default = final: prev: {
      agentos-agentd = craneLibLinux.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        cargoExtraArgs = "-p agentd";
      });
      agentos-agentctl = craneLibLinux.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        cargoExtraArgs = "-p agentctl";
      });
      agentos-egress = craneLibLinux.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        cargoExtraArgs = "-p agentos-egress";
      });
      agentos-voice = craneLibLinux.buildPackage (commonArgs // {
        inherit cargoArtifacts;
        cargoExtraArgs = "-p agentos-voice";
      });
      agentos-system-skills = final.callPackage ./packages/agentos-system-skills { };
      agentos-plymouth-theme = final.callPackage ./nix/modules/plymouth-theme { };
    };

    # --- NixOS Configurations ---
    nixosConfigurations = {
      # Dev VM with Sway desktop + OpenClaw + agentd
      agentos-dev = nixpkgs.lib.nixosSystem {
        system = linuxSystem;
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
      agentos-server = nixpkgs.lib.nixosSystem {
        system = linuxSystem;
        modules = sharedModules ++ [
          ./nix/hosts/server.nix
        ];
      };

      # Hetzner VPS (deploy via nixos-anywhere)
      agentos-hetzner = nixpkgs.lib.nixosSystem {
        system = linuxSystem;
        modules = sharedModules ++ [
          ./nix/hosts/hetzner.nix
          disko.nixosModules.disko
        ];
      };

      # Installer ISO
      agentos-iso = nixpkgs.lib.nixosSystem {
        system = linuxSystem;
        modules = sharedModules ++ [
          ./nix/hosts/iso.nix
          "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
        ];
      };
    };

    # --- Packages ---
    packages.${linuxSystem} = {
      agentd = self.overlays.default linuxPkgs linuxPkgs // { inherit (linuxPkgs) agentos-agentd; };
      default = linuxPkgs.agentos-agentd;
    };

    # --- Checks ---
    checks.${linuxSystem} = {
      agentd-clippy = craneLibLinux.cargoClippy (commonArgs // {
        inherit cargoArtifacts;
        cargoClippyExtraArgs = "--all-targets -- --deny warnings";
      });
      agentd-tests = craneLibLinux.cargoTest (commonArgs // {
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
          # Node.js (for agentos-bridge plugin)
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
          echo "AgentOS development shell"
          echo "  cargo check --workspace         - Check Rust code"
          echo "  cargo test --workspace          - Run tests"
          echo "  cargo run -p agentd -- --help   - Run agentd"
        '';
      };
    }
  );
}
