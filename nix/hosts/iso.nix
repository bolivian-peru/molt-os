# Thorox Live ISO — Boot from USB, talk to your computer
#
# Build (on a Linux machine or the Hetzner server):
#   nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage
#
# Write to USB:
#   dd if=result/iso/agentos-*.iso of=/dev/sdX bs=4M status=progress
#
# What happens when you boot:
#   1. Plymouth splash (Thorox logo, breathing animation)
#   2. Auto-login → Sway kiosk mode
#   3. Firefox opens full-screen to setup wizard (localhost:18789)
#   4. User connects WiFi, enters Anthropic API key
#   5. Thorox is alive — start chatting with your computer
#
{ config, lib, pkgs, ... }:

{
  networking.hostName = "agentos-live";
  system.stateVersion = "24.11";

  # --- Enable AgentOS (core + gateway + shell) ---
  services.agentos = {
    enable = true;
    openclaw.enable = true;
    sandbox.enable = false;
    memory.enable = true;
    shell.enable = true;
    plymouth.enable = true;
  };

  # Silent boot params are set by agentos.nix when plymouth.enable = true
  # Adding vt.global_cursor_default=0 here (not in the module)
  boot.kernelParams = [ "vt.global_cursor_default=0" ];

  # --- Live ISO user ---
  users.users.agent = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" "audio" "video" "input" ];
    initialPassword = "";
    shell = pkgs.bash;
  };

  security.sudo.wheelNeedsPassword = false;

  # --- Networking (WiFi + Ethernet) ---
  networking.networkmanager.enable = true;
  networking.wireless.enable = false; # NetworkManager handles WiFi

  # --- Audio (PipeWire) ---
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };

  # --- Fonts (clean rendering) ---
  fonts = {
    packages = with pkgs; [
      jetbrains-mono
      noto-fonts
      noto-fonts-emoji
    ];
    fontconfig.defaultFonts = {
      monospace = [ "JetBrains Mono" ];
      sansSerif = [ "Noto Sans" ];
    };
  };

  # --- System packages ---
  environment.systemPackages = with pkgs; [
    # Core tools
    vim
    git
    curl
    jq
    htop

    # Desktop
    firefox
    foot
    wl-clipboard
    pcmanfm
    networkmanagerapplet

    # For the NixOS install-to-disk flow
    gparted
    parted

    # Build tools (for running cargo if needed)
    gcc
    pkg-config
    sqlite
    openssl
  ];

  # --- Nix ---
  nix.settings = {
    experimental-features = [ "nix-command" "flakes" ];
    auto-optimise-store = true;
  };

  # --- Misc ---
  time.timeZone = "UTC";
  i18n.defaultLocale = "en_US.UTF-8";

  # --- ISO-specific: increase tmpfs for live session ---
  boot.tmp.tmpfsSize = "80%";
}
