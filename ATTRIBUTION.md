# Attribution

Every asset bundled with or committed to OpenPT is listed here with its author, source and
license. `cargo xtask assets` fails CI when a tracked asset file has no row.

**Rule:** no Avid, Pro Tools, Adobe or other proprietary icons, images, sounds or presets — ever.
Assets must be original, public domain (CC0) or permissively licensed.

## Icons

All UI icons (tools, transport, track buttons) are drawn procedurally in
`crates/ui-egui/src/icons.rs` by the OpenPT contributors (MIT OR Apache-2.0). No UI icon image
files are used. The app icon (`assets/app-icon/`, a waveform drawn as original SVG by BourrinAudio) is listed under Files below.

## Audio

The demo session's audio (drums, bass, pad) and its MIDI parts are synthesised in code in
`crates/engine/src/demo.rs` by the OpenPT contributors (MIT OR Apache-2.0). No sample files
are bundled.

## Fonts

OpenPT bundles no font files. It uses the default fonts shipped inside the `egui` crate
(Ubuntu-Light: Ubuntu Font Licence 1.0; Hack: MIT; Noto Emoji: OFL-1.1; emoji-icon-font: OFL-1.1/MIT)
and, at runtime, the operating system's own UI fonts when available (never redistributed).

## Files

| Path | Author | Source | License |
|---|---|---|---|
| `assets/app-icon/icon.svg` | BourrinAudio | original (hand-written SVG; temporary app icon) | MIT OR Apache-2.0 |
| `assets/app-icon/openpt-1024.png` | BourrinAudio | original, rendered from `icon.svg` | MIT OR Apache-2.0 |
| `assets/app-icon/openpt-macos-512.png` | BourrinAudio | original, rendered from `icon.svg` | MIT OR Apache-2.0 |
| `assets/app-icon/openpt.icns` | BourrinAudio | original, rendered from `icon.svg` | MIT OR Apache-2.0 |
| `assets/app-icon/openpt.ico` | BourrinAudio | original, rendered from `icon.svg` | MIT OR Apache-2.0 |
| `assets/app-icon/hicolor/` | BourrinAudio | original, rendered from `icon.svg` (16–512 px PNGs and the scalable SVG) | MIT OR Apache-2.0 |
| `docs/images/` | BourrinAudio | original, screenshots of OpenPT rendered by the `ui_shot` example on the demo session | MIT OR Apache-2.0 |
| `crates/transcribe/models/basic-pitch-nmp.onnx` | Spotify AB | https://github.com/spotify/basic-pitch (basic_pitch/saved_models/icassp_2022/nmp.onnx) | Apache-2.0 |
| `crates/mix/src/hrir_kemar.rs` | Bill Gardner, Keith Martin (MIT Media Lab) | https://sound.media.mit.edu/resources/KEMAR.html (diffuse-field set, 11 directions) | free use with citation |
