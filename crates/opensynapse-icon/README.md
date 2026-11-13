# opensynapse-icon

The OpenSynapse app icon, drawn in code. No image files in the repo, no dependencies.

- Motif: a synapse node with three branches (teal → blue) on a dark rounded square. Our own
  design; no Razer artwork or colours ([[ADR-0001 Clean-room reverse engineering]]).
- `render(size)`: straight RGBA, 4×4 supersampled; strokes thicken below ~24 px.
- `png`, `ico` (sizes 16–256, PNG entries), `windows_res` (compiled `.res`, icon group 1).
- `embed_windows_icon()`: called from the `build.rs` of `opensynapse`, `opensynapse-agent`
  and `opensynapse-desktop`; on Windows/MSVC it writes the `.res` to `OUT_DIR` and passes it to
  `link.exe`. The Start-menu shortcut uses the GUI exe's icon 0, so it follows automatically.
- The GUI also sets it as window icon (`ViewportBuilder::with_icon`, 64 px).

Formats are written with std only: PNG (Sub filter, zlib with one fixed-Huffman deflate block,
runs at distance 1), ICO, and the Windows resource file layout. Size: ~32 KB per exe.

Preview: `OPENSYNAPSE_ICON_OUT=<dir> cargo test -p opensynapse-icon write_preview -- --ignored`.

Not done yet: Linux launcher `Icon=` (needs a PNG under `~/.local/share/icons`), macOS `.app`.
