# Contributing to OpenPT

OpenPT aims to be a stable open-source DAW that works alongside Pro Tools sessions. Bug reports, fixes and new features are all welcome. For a large change, open an issue first so we can agree on the approach.

## Set up

You need:

- Rust 1.95 or later
- the `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- on Windows, for ASIO only: libclang, with `LIBCLANG_PATH` pointing to the folder holding `libclang.dll`

Then build and run the demo session:

```sh
git clone https://github.com/aurelienjsureau/openpt
cd openpt
cargo run --release -p openpt -- --demo
```

## Before you open a pull request

Run the same checks as the project, with installed plugins kept out of the tests:

```sh
OPENPT_NO_SYSTEM_PLUGINS=1 cargo xtask ci
```

This runs, in order: `fmt`, `clippy -D warnings`, tests, `assets`, `layers` and `wasm`.

Keep each pull request to one topic. A bug fix comes with a test that fails without it, and a crash fix always does.

## Rules for the code

**Never crash.**

- Non-test code uses no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or `unimplemented!`. Errors are `Result` and `?`, and an unfinished feature returns an "unsupported" error.
- Numbers from files, commands and control parameters are hostile: use `get()` rather than `[i]`, checked or saturating arithmetic, and capped allocations.
- The audio callback never blocks, panics or allocates in steady state.

**`unsafe` stays in the FFI modules** of `vst3-host` and `au-host`. It is forbidden everywhere else.

**Respect the layers.** `cargo xtask layers` enforces them, and nothing below the UI may depend on egui. See [`docs/architecture.md`](docs/architecture.md).

**Everything is a command.** User-visible behaviour is a command in `crates/engine/src/cmd/`, with tests. UI-only commands live in `crates/ui-egui/src/menus.rs`. Menus, shortcuts, the CLI, the control channel and MCP all dispatch the same ids.

**No code comments.** Use clear names and small functions, and explain the reasoning in the pull request.

## Audio changes

Changes to DSP must not alter the sound unless that is their purpose. If they do, say so in the pull request and describe how you measured the result.

To drive the app without a window:

```sh
cargo run -p openpt-cli -- run --demo --cmd 'ID=JSON' --bounce out.wav
```

## Interoperability and licensing

OpenPT interoperates with Pro Tools sessions and hosts VST3 and Audio Units plugins. To keep it that way:

- Never submit code, tables, artwork, icons, sounds, presets or wording taken from Pro Tools or any other proprietary product. Feature names are fine.
- Do not add AAX or anything that bypasses PACE or another protection system.
- Do not copy GPL, AGPL or LGPL code.
- Every bundled asset is original or permissively licensed and has a row in [`ATTRIBUTION.md`](ATTRIBUTION.md). `cargo xtask assets` checks it.

By contributing, you agree that your work is licensed under MIT OR Apache-2.0, like the rest of the project.
