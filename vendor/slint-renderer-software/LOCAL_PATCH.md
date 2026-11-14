# Local text rendering patches

Unmodified files come from the crates.io `i-slint-renderer-software` 1.18.1 package,
upstream revision `372cf0ee5577c3dfec309a45e7b778ba4e81b734`, directory
`internal/renderers/software`. Original license files are retained in `LICENSES/`.

The invalidation patch reads `text.text()` in the Parley `draw_text` branch.
The partial renderer must directly depend on the content even after a shaped-layout
cache entry is dropped; otherwise its tracker can lose the indirect dependency and
leave the old glyphs visible. No text cache is disabled and no repaint timer is added.

Regression: `cargo test -p kugou-lite tst_txtcache --locked --offline`.
It uses Slint's existing test hook to evict layouts, changes equal-width song text
and duration, and checks both pixel regions. Without the read, the song-title check
fails; with it both must pass. The test hook is enabled only by a dev dependency.

The second patch keeps Swash's `.hint(false)` unhinted outline scaling (matching
Parley's `quantize = false` layout so CJK characters stay aligned and uniform)
and applies a precomputed gamma `0.74` stem-contrast LUT (`STEM_GAMMA_LUT`) for
glyphs `<= 28px` in `fonts/vectorfont.rs`. The original bounded glyph cache and
grayscale coverage are retained. Check legibility at both native and 2x scale.

The third patch tracks `last_size` in `SoftwareRenderer` (`lib.rs`) so that when
the physical buffer size changes (such as across mixed-DPI monitors), the partial
rendering cache is cleared and the full frame repaints without stale DPI ghosting.
