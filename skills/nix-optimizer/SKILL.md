---
name: nix-optimizer
description: >
  Intelligent Nix store management. Analyze store usage, identify waste,
  optimize garbage collection, and track generation history.
tools:
  - shell_exec
  - memory_store
  - memory_recall
activation: auto
---

# Nix Store Optimizer

Intelligent garbage collection and store management.

## Store Analysis

```
shell_exec({ command: "du -sh /nix/store 2>/dev/null" })
shell_exec({ command: "nix-store --gc --print-dead 2>/dev/null | wc -l" })
shell_exec({ command: "nixos-rebuild list-generations 2>/dev/null | tail -20" })
```

## Smart GC Strategy

Don't just `nix-collect-garbage -d`. Be intelligent:

1. **Always keep**: current generation, last known-good, backup baseline
2. **Analyze**: which generations introduced large store additions
3. **Recommend**: targeted cleanup with estimated space recovery

```
Nix Store Report:

Store size: 28.4 GB across 47 generations

Recommended to keep:
  Gen 50 (current) — 12.1 GB active
  Gen 47 (pre-OpenSSL update) — rollback point
  Gen 42 (last backup baseline)

Safe to remove: 44 generations → reclaims ~19 GB

Large items:
  /nix/store/...-linux-6.6.12 (old kernel) — 980 MB
  /nix/store/...-chromium-121 (old browser) — 1.4 GB
  /nix/store/...-texlive-2023 (unused) — 2.1 GB

Proceed with smart cleanup?
```

## Execution

```
shell_exec({ command: "nix-collect-garbage --delete-older-than 14d" })
shell_exec({ command: "nix-store --optimise" })  # dedup hardlinks
```

## Post-Cleanup

```
memory_store({
  summary: "Nix GC: freed 19GB, kept gens 42,47,50",
  detail: "Store reduced from 28.4GB to 9.3GB. 44 generations removed.",
  category: "system.config",
  tags: "nix,garbage-collection,optimization"
})
```
