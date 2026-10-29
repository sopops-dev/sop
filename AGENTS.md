# Notes for coding agents

## Writing sop files (instructions, SOPs, agents)

Read [FORMAT.md](FORMAT.md) before creating or editing any file in a sop folder. It is the complete format reference and ends with a checklist. In a project that uses sop, `sop guide` prints the same reference.

## Working on this repo

- Rust (stable, 1.80+): `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Run the CLI from source with `cargo run -- <command>`, or `cargo run -- -C <folder>` to compile a folder.
- `src/model.rs` is the source of truth for the format. Markdown SOPs (`src/sopfile.rs`) are read into the same mapping a YAML SOP holds, then parsed by `model::parse_sop`; `sopfile.rs` also writes the canonical text for `sop fmt` and `sop convert`, which must never change a built prompt or tool.json. `spec/*.schema.json` is the published JSON Schema; when you change a field, update `spec/` and FORMAT.md to match (a test checks the field lists against `spec/`).
- The format of sopc v0.0.8 and earlier (`bases/`, `inherits`, `exclude`, block targeting, `position`, `sop_order`) is read only by `src/migrate.rs`, for `sop migrate` and for building a `--against` ref from before it. Keep it there; the rest of the code knows only the current format, and reports old fields as `old_format`. `tests/fixtures/old-format/` holds old folders with the prompts the previous release built from them.
- `lock.json` block hashes are sha256 of each block's canonical JSON (`canonical_json` in `src/model.rs`: fields in declared order, no spaces, non-ASCII kept). Changing field order or JSON encoding changes every hash, so treat it as a breaking change.
- `tests/fixtures/restaurants/expected/` is golden output. If a change to the built prompts is intended, regenerate it with `cargo run -- -C tests/fixtures/restaurants/sops -o tests/fixtures/restaurants/expected` and review the diff.
- `examples/livekit-restaurant/sops/` must stay valid, formatted (`sop fmt --check`) and its `build/` current; tests check all three. Run `cargo run -- --dir examples/livekit-restaurant/sops` after changing how prompts are built.
- `skills/` holds skills shipped to users (`sop skills install`), and FORMAT.md is printed by `sop guide`; both are embedded in the binary (`include_str!` in `src/main.rs`). Keep them in sync with the CLI's commands and output.
- Releases: set `version` in `Cargo.toml`, then push a matching `v*` tag; `.github/workflows/release.yml` builds every target and creates a draft release with the archives and `checksums.txt` that `install.sh` downloads.
