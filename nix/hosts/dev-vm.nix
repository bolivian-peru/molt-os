# AgentOS Dev VM — QEMU virtual machine with Sway desktop
# Build: nix build .#nixosConfigurations.agentos-dev.config.system.build.vm
# Run:   ./result/bin/run-agentos-dev-vm -m 4096 -smp 4
{ config, lib, pkgs, ... }:

{
  # --- System Identity ---
  networking.hostName = "agentos-dev";
  system.stateVersion = "24.11";

  # --- Enable AgentOS ---
  services.agentos = {
    enable = true;
    openclaw.enable = true;
    sandbox.enable = true;
    memory.enable = true;
    voice.enable = true;
    plymouth.enable = true;
    shell.enable = true;
  };

  # --- VM Configuration ---
  virtualisation.vmVariant = {
    virtualisation = {
      memorySize = 4096;
      cores = 4;
      graphics = true;
      qemu.options = [
        "-vga virtio"
        "-display gtk,gl=on"
      ];
      diskSize = 20480;  # 20GB
      forwardPorts = [
        { from = "host"; host.port = 18789; guest.port = 18789; }  # OpenClaw Gateway
      ];
    };
  };

  # Desktop shell is managed by services.agentos.shell (agentos-shell.nix)

  # --- Auto-login User ---
  users.users.agent = {
    isNormalUser = true;
    description = "AgentOS User";
    extraGroups = [ "wheel" "networkmanager" "video" "audio" ];
    initialPassword = "agentos";
    shell = pkgs.zsh;
  };

  programs.zsh.enable = true;

  # --- Security ---
  security.sudo.wheelNeedsPassword = false;

  # Additional packages beyond what the shell module provides
  environment.systemPackages = with pkgs; [
    kitty           # terminal (power user escape hatch)
    networkmanagerapplet
  ];

  # --- Audio ---
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };

  # --- Networking ---
  networking.networkmanager.enable = true;

  # --- Fonts ---
  fonts.packages = with pkgs; [
    noto-fonts
    noto-fonts-cjk-sans
    noto-fonts-emoji
    (nerdfonts.override { fonts = [ "FiraCode" "JetBrainsMono" ]; })
  ];

  # Sway kiosk config and Waybar are managed by agentos-shell.nix

  # --- Nix Settings ---
  nix = {
    settings = {
      experimental-features = [ "nix-command" "flakes" ];
      auto-optimise-store = true;
    };
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 14d";
    };
  };

  # --- Boot ---
  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
}
