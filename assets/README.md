# Application icons

Loom's canonical icon assets now live in `../icons`.

`../icons/app.ico` is the standard Windows application icon used by the
executable, taskbar, Start menu, window title bar, and Alt+Tab. It contains
32-bit transparent images at 16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72,
80, 96, 128, and 256 pixels.

Notification-area variants are provided separately because Tray icons are
displayed at much smaller sizes:

- `../icons/tray-color.ico` — simplified color icon.
- `../icons/tray-light.ico` — light icon for dark taskbars.
- `../icons/tray-dark.ico` — dark icon for light taskbars.

The canonical transparent artwork and exported PNG files are retained in
`../icons/source`. The application ICO is generated from
`../icons/source/app-icon-transparent-1024.png`:

```console
node ./scripts/generate-app-icon.js
```
