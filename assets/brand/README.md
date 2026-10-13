# lull brand

The sleeping cat and the "lull" wordmark. Colours: lavender `#EEEDF6` and indigo `#272744`.
The SVGs are the masters; do not edit their paths. Each PDF and the `.icns` were made once
from the SVG of the same name, so the app loads vectors natively and builds need no
converter:

```sh
rsvg-convert -f pdf <name>.svg -o <name>.pdf
# app-icon.icns: rsvg-convert app-icon.svg to 16–1024 px PNGs in an .iconset, then
iconutil -c icns AppIcon.iconset -o app-icon.icns
```

| File | Use |
|---|---|
| `symbol.svg`, `symbol-currentcolor.svg` | The cat alone, indigo or in the current text colour. |
| `logo-horizontal.svg` / `.pdf` | Cat and wordmark side by side; the menu-bar panel header. |
| `wordmark.svg` / `.pdf` | "lull" alone. |
| `app-icon.svg` / `.pdf`, `app-icon.icns` | The cat on an indigo rounded tile; the app's bundle icon. |
| `lull-micro-{16,20,24}.svg` / `.pdf`, `-currentColor.svg` | The cat redrawn for 16, 20 and 24 px. The menu bar uses `lull-micro-20` as a template image. |
| `header/header-{dark,light}@{2x,4x}.png` | The night-sky skyline behind the panel header: a 360 × 100 pt band without the logo, opaque over the panel colour (`#20203A` dark, `#BDBAD2` light, each the colour of its darkest buildings) so it fades into the panel. Raster masters, copied into the app as they are. |
| `reference/brand-sheet.png` | The approved sheet: logo on light and dark, app icon and menu-bar use. Reference only, not shipped. |

All artwork is single-colour with the details cut out (`fill-rule="evenodd"`), so it can be
drawn as a template and tinted: indigo on light backgrounds, lavender on dark ones.

The terminal draws its own half-block rendering of the logo (`LOGO` in
`crates/cli/src/ui/mod.rs`).
