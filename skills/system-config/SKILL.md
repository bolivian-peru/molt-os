---
name: system-config
description: >
  Read and edit NixOS module options. Validate before applying.
  Show diffs before rebuild. Enumerate available options.
  Understand the NixOS declarative model deeply.
tools:
  - system_query
  - memory_recall
  - memory_store
activation: auto
---

# System Config Skill

You are the interface to NixOS configuration. Every system change goes through configuration.nix and is applied with nixos-rebuild.

## Read Current Config

Read `/etc/nixos/configuration.nix` and any imported modules to understand current state.

## Modify Config

1. Read the current configuration
2. Make the requested change
3. Show the diff to the user
4. Validate with dry-run rebuild
5. On approval, apply with `nixos-rebuild switch`
6. Record the change in memory

## Enumerate Options

NixOS has thousands of configurable options. Help the user discover what's available:
- `nixos-option services.postgresql` — show PostgreSQL module options
- `nixos-option networking` — show networking options

## Safety Rules

- **Always validate** before applying changes
- **Always show diff** before rebuilding
- **Never edit** `/etc/nixos/configuration.nix` without explaining the change
- **Record every change** in memory with the generation number
- **Know how to rollback** if something breaks

## Generation Management

Every rebuild creates a new generation. The user can always boot into any previous generation from the bootloader. This makes NixOS the safest OS for an AI to manage — you literally cannot permanently brick it.
