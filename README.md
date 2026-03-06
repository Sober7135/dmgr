# dmgr

`dmgr` is a local-first Docker development manager for repeatable build and run workflows.

## Concepts

Each entry lives under `DMGR_ROOT/entries/<name>` and contains:

- `entry.toml`: metadata such as `workspace`, `managed`, `autobuild`, and shell
- `build.sh`: the build workflow
- `run.sh`: the default run workflow
- `cmd-overrides/`: path-scoped run command overrides
- `workspace/`: the default managed workspace and Docker build context

By default `DMGR_ROOT` is resolved as `$HOME/.config/dmgr`. You can override it with `--root` or the `DMGR_ROOT` environment variable.

`dmgr` creates managed entries by default. In managed mode, the workspace is `DMGR_ROOT/entries/<name>/workspace`. If you want to reuse an existing project directory, pass `--workspace /path/to/project`.

`dmgr run <name>` resolves the run command in two steps:

- if the current `cwd` exactly matches a stored override path, use that override
- otherwise fall back to the default `run.sh`

Each run prints the chosen scope before executing, for example `default` or the matched absolute path.

## Commands

```bash
dmgr entry create dev --autobuild
dmgr entry create dev --workspace /path/to/project
dmgr entry list
dmgr file edit dev
dmgr script edit dev
dmgr cmd edit dev
dmgr cmd edit dev --cwd
dmgr cmd edit dev --workspace /path/to/project
dmgr build dev
dmgr rm dev
dmgr rm dev --yes
dmgr run dev
dmgr build --autobuild
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
