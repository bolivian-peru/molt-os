---
name: natural-language-config
description: >
  Translate natural language requests into NixOS configuration changes.
  Show diffs before applying. Use nixos-rebuild for atomic deploys.
  Every change is rollbackable and logged.
tools:
  - shell_exec
  - file_read
  - file_write
  - memory_store
  - memory_recall
  - event_log
activation: auto
---

# Natural Language NixOS Config Skill

Users describe what they want in plain English. You write the NixOS config.

## Workflow

1. **Understand the request**
   "Set up PostgreSQL with nightly backups"
   "Install and configure Caddy as a reverse proxy for port 3000"
   "Enable fail2ban to block brute force SSH"

2. **Read current config**
   ```
   file_read({ path: "/etc/nixos/configuration.nix" })
   ```

3. **Generate the NixOS module**
   Write idiomatic NixOS configuration. Use `services.*`, `networking.*`, `security.*` etc.
   Prefer declarative NixOS options over imperative scripts.

4. **Show the diff to the user**
   Present EXACTLY what will change. Use a code block with +/- diff markers.
   Explain each change in plain English.

5. **Wait for approval**
   NEVER apply changes without user confirmation.
   "Here's what I'll add to your NixOS config. Apply it? [Y/n]"

6. **Apply atomically**
   ```
   shell_exec({ command: "nixos-rebuild switch" })
   ```

7. **Verify**
   Check the service started correctly.
   ```
   service_status({ service: "postgresql" })
   ```

8. **Log everything**
   ```
   memory_store({
     summary: "Installed PostgreSQL with nightly pg_dump backups",
     detail: "Added services.postgresql, services.postgresqlBackup to configuration.nix. Generation 49.",
     category: "system.config",
     tags: "nixos,postgresql,config-change"
   })
   ```

## NixOS Config Conventions

- Configuration goes in `/etc/nixos/configuration.nix` or imported modules
- Use `mkIf`, `mkOption`, `mkDefault` when extending existing config
- Prefer `services.X.enable = true;` patterns
- For packages: add to `environment.systemPackages`
- For firewall: `networking.firewall.allowedTCPPorts`
- For users: `users.users.X`

## Example Interaction

User: "Set up nginx to serve a static site from /var/www"

Response:
```
I'll add nginx to your NixOS config. Here's the change:

  services.nginx = {
    enable = true;
    virtualHosts."default" = {
      root = "/var/www";
      locations."/" = {
        index = "index.html";
      };
    };
  };
  networking.firewall.allowedTCPPorts = [ 80 443 ];

This will:
- Install and start nginx
- Serve /var/www on port 80
- Open ports 80 and 443 in the firewall

Apply this? The change is atomic and rollbackable.
```

## Rollback

If a rebuild breaks something:
```
shell_exec({ command: "nixos-rebuild switch --rollback" })
```
Then investigate what went wrong.

## Critical rules

- ALWAYS show the diff before applying
- ALWAYS wait for user approval
- ALWAYS verify after rebuild
- NEVER edit /etc/nixos files without reading them first
- Store every config change in memory for time-travel debugging
