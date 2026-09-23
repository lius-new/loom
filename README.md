# leditor

A minimal text editor written in Rust, using the [lgui](https://crates.io/crates/lgui) GUI library.

## Project layout

```
leditor/
├── Cargo.toml   # package manifest + lgui dependency
└── src/
    └── main.rs  # the whole editor
```

## Features

- Plain text editing (UTF-8 safe)
- File tree icons selected by exact filename and longest matching extension
- `Enter` — new line, `Tab` — indent (4 spaces)
- `Backspace` / `Delete` — remove character before/after caret
- Arrow keys — move caret left/right/up/down
- `Home` / `End` — jump to start/end of line
- A visible caret (only while the editor has focus) and a status bar showing `Ln X, Col Y`

The editor is keyboard-driven: click the dark editing area to focus it, then type.

## File icons

The entire workspace tree uses a focused subset of the open-source Material
Icon Theme, including closed folders, open folders, and file types. Exact
filenames such as `Cargo.toml` and `Dockerfile` take priority over extensions;
compound extensions are matched longest-first, and unknown files use a generic
document icon. Attribution, source revision, and the upstream MIT license are
stored with the SVG assets under
`assets/icon-themes/material/`.

## Build & run

```sh
cargo build --release
cargo run --release
```

`lgui` uses Skia for rendering and Winit for the window. The renderer is the
**software** Skia backend (`renderer-skia-software`), which presents via
`softbuffer`. `skia-safe` downloads a prebuilt Skia archive automatically at
build time.

## System dependencies (Debian / Ubuntu)

```sh
sudo apt install build-essential \
    libfreetype-dev libfontconfig-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgles2-mesa-dev \
    libwayland-dev libxkbcommon-dev
```

A running X11 or Wayland desktop session is required to open the window.
(On WSL2, enable WSLg and install the packages above inside the distro.)
