# cactus

Finds `build/`, `node_modules/`, `target/`, and other cache directories across your repos, then deletes them -- but only if they're gitignored. If git doesn't say to ignore it, cactus won't touch it.

```
$ cactus ~/code
apps/dashboard
  node_modules  309.5 MiB
apps/backend
  build  12.2 MiB
  .gradle  986 KiB
libs/rust-core
  target  1.3 GiB
tools/cli
  node_modules  61.9 MiB

12 dirs, 1.7 GiB reclaimable
Purge? [y/N]
```

## Install

```sh
cargo install --git https://github.com/iamkaf/cactus
```

## Usage

```
cactus <path>              # scan and prompt before deleting (default depth: 3)
cactus -L 5 <path>         # look deeper for repos
cactus -n <path>           # dry run, just list what it finds
cactus -y <path>           # skip the confirmation prompt
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--depth` | `-L` | `3` | Max directory depth to search for repos |
| `--dry-run` | `-n` | off | Show what would be deleted without deleting |
| `--yes` | `-y` | off | Skip confirmation prompt |

## What it looks for

| Directory | Language/tool |
|---|---|
| `build/` | Gradle, generic |
| `.gradle/` | Gradle |
| `bin/`, `obj/` | .NET, generic |
| `node_modules/` | Node.js |
| `target/` | Rust, Maven |
| `__pycache__/` | Python |
| `.mypy_cache/`, `.pytest_cache/`, `.ruff_cache/`, `.tox/` | Python tooling |

## How it works

1. Walks directories up to the specified depth looking for `.git` folders
2. Scans each repo in parallel using libgit2 (via [git2](https://crates.io/crates/git2)) and [rayon](https://crates.io/crates/rayon)
3. Checks each candidate directory against the repo's `.gitignore` rules -- only gitignored directories are marked for deletion
