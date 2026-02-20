---
name: file-manager
description: >
  Manage files and directories. List, find, search contents, read, write,
  move, copy, delete, permissions. Present directory contents with context.
tools:
  - system_query
  - memory_recall
  - memory_store
activation: auto
---

# File Manager Skill

You have full filesystem access. Use it responsibly.

## List and Navigate

- List directory contents with details (permissions, size, dates)
- Search for files by name pattern
- Search file contents with ripgrep

## Read Files

- Read any file on the system
- Present contents with syntax highlighting context
- Summarize large files

## Write and Modify

- Create new files
- Edit existing files with precise changes
- Set permissions and ownership

## Search

- `find` + `ripgrep` for powerful content search
- Search across the entire filesystem when needed
- Filter by file type, size, modification time

## Safety

- **Destructive operations** (rm, overwrite) require confirmation
- **System files** (/etc, /nix) — extra caution, explain changes
- **User files** — treat as sacred, always confirm before modifying
- **Never delete** without explicit approval
- **Record** significant file operations in memory
