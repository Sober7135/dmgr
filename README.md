# dmgr

`dmgr` is a local-first Docker development manager for repeatable build and run workflows.

## Concepts

Each entry lives under `DMGR_ROOT/entries/<name>` and contains:

- `entry.toml`: metadata such as `workspace`, `managed`, `autobuild`, `depends_on`, and shell
- `build.sh`: the build workflow
- `run.sh`: the default run workflow
- `cmd-overrides/`: path-scoped run command overrides
- `workspace/`: the default managed workspace and Docker build context

By default `DMGR_ROOT` is resolved as `$HOME/.config/dmgr`. You can override it with `--root` or the `DMGR_ROOT` environment variable.

`dmgr` creates managed entries by default. In managed mode, the workspace is `DMGR_ROOT/entries/<name>/workspace`. If you want to reuse an existing project directory, pass `--workspace /path/to/project`.

`dmgr import <path>` imports external Docker workspaces. If `<path>` itself contains a `Dockerfile`, `dmgr` imports it as a single external entry named after the directory. Otherwise `dmgr` scans the immediate child directories under `<path>` and imports each child that contains a `Dockerfile`.

During import, `dmgr` also inspects `Dockerfile` `FROM ...` references. If a base image name matches another imported entry or an existing local entry, `dmgr` stores it in `depends_on`.

Current limitation for imported git repositories:

- `workspace` points at the external directory, so `dmgr file edit <name>` updates the source repository `Dockerfile`
- `build.sh` is copied into `DMGR_ROOT/entries/<name>/build.sh` during import, so `dmgr script edit <name>` updates the local dmgr copy instead of the source repository
- `dmgr` does not currently provide native git sync helpers; if you need changes tracked by the source repository, update files inside the repo directly and use git there

This ownership model may be revised later so imported entries can use source `build.sh` files more directly.

Build commands use the recorded dependency graph:

- `dmgr build <name>` builds the requested entry after its dependencies
- `dmgr build --autobuild` builds only `autobuild = true` entries, still respecting dependencies
- `dmgr build-all` builds every recorded entry in dependency order

Independent build nodes can run in parallel. By default `dmgr` uses the local available parallelism. You can override the worker count with `DMGR_BUILD_JOBS=<n>`.

Build commands no longer stream raw `docker build` output directly to the terminal. Instead they print status lines such as `start`, `done`, `fail`, and `skip`. Each build writes its full stdout and stderr to `DMGR_ROOT/entries/<name>/build.last.log`.

Generated scripts should use `#!/usr/bin/env sh` or `#!/usr/bin/env bash`. The default configured shell is `sh`. If an older entry still records a missing absolute shell path such as `/bin/sh`, `dmgr` falls back to the basename and resolves it from `PATH`.

`dmgr run <name>` resolves the run command in two steps:

- if the current `cwd` exactly matches a stored override path, use that override
- otherwise fall back to the default `run.sh`

Each run prints the chosen scope before executing, for example `default` or the matched absolute path.

## Commands

```bash
dmgr entry create dev --autobuild
dmgr entry create dev --workspace /path/to/project
dmgr import ../Dockerfile
dmgr import ../Dockerfile/dev
dmgr entry list
dmgr file edit dev
dmgr script edit dev
dmgr cmd edit dev
dmgr cmd edit dev --cwd
dmgr cmd edit dev --workspace /path/to/project
dmgr build dev
dmgr build graphar-arch-dev
dmgr build --autobuild
dmgr build-all
dmgr rm dev
dmgr rm dev --yes
dmgr run dev
dmgr init systemd
dmgr init openrc
```

`dmgr rm <name>` asks for confirmation before removing `DMGR_ROOT/entries/<name>`. Use `--yes` for non-interactive removal. For external workspaces created with `--workspace`, the workspace directory itself is left untouched.

## Boot Integration

Render a `systemd` unit:

```bash
dmgr init systemd --dmgr-bin /usr/local/bin/dmgr > dmgr-autobuild.service
```

Render an `openrc` service:

```bash
dmgr init openrc --dmgr-bin /usr/local/bin/dmgr > dmgr-autobuild
```
