# OpenPT

A digital audio workstation in pure Rust, modelled on the Pro Tools workflow: Edit and Mix windows,
inserts A–J (multichannel or multi-mono), sends, busses, VCAs, playlists, clip gain, automation
(Read/Touch/Latch/Write/Trim), MIDI editing and recording, bounce.

Plugins: VST3 (Windows, macOS, Linux) and Audio Units (macOS). Third-party plugins are scanned in
child processes, so one that crashes or hangs while loading is skipped instead of taking the app
down.

## Build

```
cargo build --release -p openpt -p openpt-cli
```

ASIO (Windows): needs libclang (`LIBCLANG_PATH` pointing to the folder holding `libclang.dll`).

```
cargo build --release -p openpt --features asio
```

## Run

- `openpt [--demo] [session.openpt | audio files…]`
- `openpt-cli run --demo --cmd 'mix.volume={"track":"Kick","db":-6}' --bounce out.wav`
- `openpt-cli commands` lists every command; the same ids drive the menus, the CLI, the control
  channel (`openpt --control <port>`, see `docs/control-protocol.md`) and the MCP server.

## Tests

```
OPENPT_NO_SYSTEM_PLUGINS=1 cargo test --workspace --exclude openpt-web
```

`OPENPT_NO_SYSTEM_PLUGINS` keeps the tests away from the plugins installed on the machine.

## Layout

| Crate | Role |
|---|---|
| `time`, `audio-io`, `midi`, `video` | timebases and tempo maps; audio files and peaks; MIDI files; video |
| `dsp` | built-in plugins, offline processing, meters |
| `model` | the session document |
| `mix` | the mix engine (realtime and bounce), multi-mono inserts |
| `engine`, `playback` | commands, undo, editing, import/export; audio and MIDI devices |
| `vst3-host`, `au-host` | plugin hosting (the only crates allowed `unsafe`) |
| `automation` | MCP server and control channel |
| `ui-kit`, `ui-egui` | interface |

## License

MIT OR Apache-2.0. See `NOTICE`, `LICENSE-MIT` and `LICENSE-APACHE`.
