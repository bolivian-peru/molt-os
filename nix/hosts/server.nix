# osModa Server — Headless configuration
{ config, lib, pkgs, ... }:

{
  networking.hostName = "osmoda-server";
  system.stateVersion = "24.11";

  services.osmoda = {
    enable = true;
    openclaw.enable = true;
    sandbox.enable = true;
    memory.enable = true;

    # MCP servers — extend the agent with external tools
    mcp.enable = true;
    mcp.servers.pageindex = {
      enable = true;
      command = "npx";
      args = [ "-y" "@pageindex/mcp" ];
      allowedDomains = [ "api.pageindex.ai" "chat.pageindex.ai" ];
    };
  };

  # Headless — no desktop
  # Gateway accessible on localhost only
  # Use SSH or reverse proxy to access remotely

  users.users.agent = {
    isNormalUser = true;
    description = "osModa Service Account";
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keys = [
      # Add your SSH public key here
    ];
  };

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      PermitRootLogin = "no";
    };
  };

  networking.firewall.allowedTCPPorts = [ 22 ];

  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  boot.loader.systemd-boot.enable = true;
  boot.loader.efi.canTouchEfiVariables = true;
}
