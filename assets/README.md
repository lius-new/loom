# Application icon

`app-icon.ico` is Loom's standard Windows application icon. It contains
32-bit transparent images at 16, 24, 32, 48, 64, 128, and 256 pixels.

The canonical artwork is retained as the 256 x 256 RGBA PNG at
`source/app-icon.png`. Regenerate the ICO after changing that source image:

```console
node ./scripts/generate-app-icon.js
```
