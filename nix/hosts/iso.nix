# AgentOS ISO — Installer image
# Build: nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage
{ config, lib, pkgs, ... }:

{
  networking.hostName = "agentos-installer";
  system.stateVersion = "24.11";

  services.agentos = {
    enable = true;
    openclaw.enable = true;
    sandbox.enable = false;  # No sandbox in installer
    memory.enable = false;   # No persistent memory in installer
  };

  # Installer boots to a minimal Sway session with the agent ready
  programs.sway.enable = true;

  services.greetd = {
    enable = true;
    settings = {
      default_session = {
        command = "${pkgs.sway}/bin/sway";
        user = "nixos";
      };
    };
  };

  users.users.nixos = {
    isNormalUser = true;
    extraGroups = [ "wheel" "networkmanager" ];
    initialPassword = "";
    shell = pkgs.bash;
  };

  security.sudo.wheelNeedsPassword = false;

  environment.systemPackages = with pkgs; [
    kitty
    firefox
    gparted
    networkmanagerapplet
  ];

  networking.networkmanager.enable = true;

  nix.settings.experimental-features = [ "nix-command" "flakes" ];
}
