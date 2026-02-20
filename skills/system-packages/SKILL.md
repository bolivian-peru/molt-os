---
name: system-packages
description: >
  Manage NixOS packages declaratively. Search, install (via configuration.nix rebuild),
  remove, rollback, and list generations. Understands the NixOS declarative model.
tools:
  - system_query
  - system_health
  - memory_recall
  - memory_store
activation: auto
---

# System Packages Skill

You manage packages through NixOS's declarative model. Every package change edits configuration.nix and triggers a rebuild. This means every change is atomic, reproducible, and rollbackable.

## Search Packages

When the user asks to install something, first search for the correct package name.

## Install Packages

1. Edit `/etc/nixos/configuration.nix` to add the package to `environment.systemPackages`
2. Validate with a dry-run rebuild
3. Show the user what will change
4. On approval, rebuild with `nixos-rebuild switch`
5. Log the installation to memory

**Always use declarative NixOS config.** Never suggest `nix-env -i` — it breaks the declarative model.

## Remove Packages

1. Remove from `environment.systemPackages` in configuration.nix
2. Rebuild
3. Optionally garbage collect old generations

## Rollback

NixOS keeps every generation. To rollback:
- List generations: `nixos-rebuild list-generations`
- Switch to previous: `nixos-rebuild switch --rollback`

## Remember

After every package operation, store what happened:
```
memory_store({
  summary: "Installed postgresql-16",
  detail: "Added services.postgresql.enable = true and services.postgresql.package = pkgs.postgresql_16 to configuration.nix. Rebuild generation 45.",
  category: "system.package",
  tags: ["postgresql", "install", "generation-45"]
})
```
