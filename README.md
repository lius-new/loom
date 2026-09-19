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
- `Enter` — new line, `Tab` — indent (4 spaces)
- `Backspace` / `Delete` — remove character before/after caret
- Arrow keys — move caret left/right/up/down
- `Home` / `End` — jump to start/end of line
- A visible caret (only while the editor has focus) and a status bar showing `Ln X, Col Y`

The editor is keyboard-driven: click the dark editing area to focus it, then type.

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
