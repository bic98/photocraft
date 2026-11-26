# PAULGA Photo app icon

The **PAULGA mark**: the white "P" built from rounded bars on a black field
(`paulga-logo.svg`, PAULGA's own work). The same mark is the app icon of the other PAULGA products.

## Geometry

The mark fills a 512-unit tile (`viewBox="0 0 512 512"`), a rounded square with `rx=112`, no
border. The macOS renders pad the tile onto Apple's 824/1024 icon grid; the Windows and Linux
renders use the whole tile. The file names keep `photocraft` and `ai.storyteller.photocraft`
because the build scripts, packaging and the desktop entry refer to them.

## Provenance and licence

`paulga-logo.svg` is PAULGA's own work, supplied by the product's owner; every other file here is
rendered from it. It is a PAULGA brand asset, not MIT OR Apache-2.0 material: see `LICENSE.txt`.
The PhotoCraft icon these files replaced is in the repository history.

## Files

- `paulga-logo.svg`: the mark as supplied (160-unit square, no rounding).
- `photocraft.svg`, `photocraft-small.svg`: the mark on the rounded 512-unit tile; the second is
  also the hicolor scalable icon.
- `photocraft-1024.png`: 1024 px render on the macOS grid.
- `photocraft.icns`: macOS bundle icon (`CFBundleIconFile`).
- `photocraft.ico`: Windows icon, 16–256 px, embedded in the `.exe` by `apps/photocraft/build.rs`.
- `hicolor/<size>/apps/ai.storyteller.photocraft.png` (16–512) and `hicolor/scalable/...svg`:
  Linux icon theme.

The app also sets the window icon and Wayland app ID at runtime (`apps/photocraft/src/app_icon.rs`,
`main.rs`). On Windows the taskbar shows the icon of the Start Menu shortcut that launches the
exe, so the MSI's `<Icon Id>` keeps the `.exe` extension (ICE50); `app_icon.rs` documents the
details and tests the `.ico` sizes and the WiX icon references.

## How these files were made

The mark is rectangles and ellipses only, so the PNG, ICO and ICNS files were drawn from its
shapes with Pillow 12.3 at 4096 px, clipped to the rounded tile and scaled down (Lanczos) to each
size. `packaging/icons.sh` renders the same set from `photocraft.svg` with `resvg` (plus
`iconutil` on macOS; the `.ico` is packed by `cargo xtask ico`); it has not been run on these files.

## Nightly channel

The nightly files (`photocraft-nightly.png`, `photocraft-nightly-1024.png`, the ICO and the ICNS)
are the same mark with the field changed from black to the deep blue the nightly channel already
used (`#22395d`); the white shapes, the tile and the Mac padding are unchanged. There is no letter
badge. Output hashes are in [`nightly-provenance.json`](nightly-provenance.json).
