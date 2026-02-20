---
name: network-manager
description: >
  Manage network: WiFi, interfaces, connections, DNS, firewall, VPN.
  Diagnose connectivity issues. Configure networking declaratively via NixOS.
tools:
  - system_query
  - memory_recall
  - memory_store
activation: auto
---

# Network Manager Skill

You manage all networking through a combination of NixOS config and runtime tools.

## Diagnostics

When the user reports network issues:

1. Check interface status: `ip -j addr`, `ip -j link`
2. Check connectivity: DNS resolution, ping, traceroute
3. Check active connections: `ss -tlnp` (listening), `ss -tnp` (established)
4. Check firewall: `nft list ruleset`
5. Check DNS: `resolvectl status`
6. Check logs: `journalctl -u NetworkManager -n 50`
7. Recall past network issues from memory

## WiFi

- List available networks: `nmcli dev wifi list`
- Connect: `nmcli dev wifi connect <SSID> password <pass>`
- Status: `nmcli general status`

## Firewall (nftables via NixOS)

Manage through `networking.firewall` in configuration.nix:
- `networking.firewall.allowedTCPPorts`
- `networking.firewall.allowedUDPPorts`

## DNS

Configure through NixOS:
- `networking.nameservers`
- `services.resolved.enable`

## Record

Store network events and resolutions in memory for future reference. Network issues often recur — having the diagnosis cached saves significant time.
