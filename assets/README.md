# Brand assets

These are the Kora project's own brand art (the fox mascot), owned by the Kora organisation. They are the same
artwork as the website icon (`public/icon-512.png`) and the Discord avatar.

- `kora-logo-512.png`: 512x512 master (copied from the website's `public/icon-512.png`).
- `ico_16.png`, `ico_32.png`, `ico_48.png`: hand-prepared small renders (face crop, tuned for small sizes).
- `kora.ico`: multi-image icon with PNG-compressed entries 16/32/48 (the hand-prepared files) and
  64/128/256 (downscaled from the master). Rebuild it with `python3 assets/make_ico.py` (needs `ffmpeg`).

Used as the `kora.exe` icon (`crates/kora-cli/build.rs`), the Start Menu / Desktop shortcut icon and the
Apps & features icon on Windows, and as the `.desktop` icon on Linux (`kora-logo-512.png` is installed next to
the program under `/opt/kora`).
