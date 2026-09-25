# NodeLite vector logo

Transparent, self-contained SVG redraws of the existing NodeLite artwork. The
server, four nodes, heartbeat, and wordmark preserve the original composition.
Raster texture and shadows are omitted; geometry and stroke widths are regularized.
The light-background palette has stronger contrast than the original pale bitmap.
The wordmark is drawn as paths, so no font installation or external assets are needed.

| File | Use |
| --- | --- |
| `nodelite-light.svg` | Full logo on light backgrounds |
| `nodelite-dark.svg` | Full logo on dark backgrounds |
| `nodelite-icon-light.svg` | Icon only on light backgrounds |
| `nodelite-icon-dark.svg` | Icon only on dark backgrounds |

`light` / `dark` describe the intended surface, not an embedded background.
Every SVG has a transparent canvas and transparent server interiors. The complete
logo is intended for larger placements; prefer the icon for small UI elements.

Open `preview.html` to compare the original bitmap with both vector palettes and
inspect the icons on a checkerboard. The preview backgrounds are not part of the
SVG files. Copies in `nodelite-server/web/public/assets/` are used by the panel;
the documentation site keeps its own copy under `docs/assets/`.
