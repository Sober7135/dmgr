# Repository Guidelines

## Project Structure

- `src/lib.rs` is the main application layer. It handles command dispatch, entry management, script execution, and path resolution.
- `src/cli.rs` defines the `clap` command model. Add new subcommands and flags here first.
- `src/entry.rs` contains data models and path rules for entries, workspaces, and command overrides.
- `src/editor.rs` resolves `$VISUAL` / `$EDITOR`; `src/init.rs` renders `systemd` and `openrc` templates.
- `tests/cli.rs` holds the main integration tests for CLI behavior, file layout, and script execution.

## Build, Test, and Development Commands

- `cargo build`: compile `dmgr` and catch compile-time issues quickly.
- `cargo run -- <args>`: run the CLI locally, for example `cargo run -- entry list`.
- `cargo fmt`: format Rust code before review or commit.
- `cargo clippy -- -D warnings`: run lints and treat all warnings as errors.
- `cargo test`: run unit and integration tests.
- `cargo fmt && cargo clippy -- -D warnings && cargo test`: recommended pre-commit check.

## Coding Style and Naming

- Follow standard Rust conventions: 4-space indentation, `snake_case` for functions and variables, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Prefer small, focused functions. Keep CLI parsing, domain modeling, and execution logic separated.
- Avoid unnecessary abstraction layers; this repository currently favors a local-first, single-binary design.
- Run `cargo fmt` and `cargo clippy -- -D warnings` before submitting changes to keep style and lint quality consistent.

## Testing Guidelines

- Add integration coverage in `tests/cli.rs` for non-trivial behavior changes; keep pure model or serialization tests close to the module.
- Use behavior-based test names such as `run_uses_cwd_override_when_present`.
- Avoid depending on a real Docker daemon in tests. Prefer temp directories, stub binaries, and small shell scripts.

## Commit and Pull Request Guidelines

- The repository does not yet have commit history, so use short imperative commit messages such as `add cwd cmd override support`.
- Pull requests should explain the goal, key design choices, test results, and any compatibility impact on entry layout or config files.
- Update `README.md` whenever CLI behavior or directory structure changes.

## Configuration and Architecture Notes

- The default root directory is `$HOME/.config/dmgr`; override it with `--root` or `DMGR_ROOT`.
- `run.sh` is the default run command, while `cmd-overrides/` stores path-scoped overrides. Preserve the `default` fallback behavior when changing run resolution logic.
