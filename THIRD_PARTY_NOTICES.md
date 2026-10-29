# Bundled font notices

Rastera's UI and canvas text use the default font definitions from epaint / epaint_default_fonts 0.31.1 (https://github.com/emilk/egui). Font bytes are bundled by that dependency, not embedded in project documents. Canvas text is editable Unicode text plus style and box geometry; glyph/raster/GPU caches are not serialized.

- Ubuntu Light: Ubuntu Font Licence 1.0 — [full notice](docs/licenses/fonts/UFL.txt).
- Hack Regular: MIT and Bitstream Vera notices — [full notices](docs/licenses/fonts/Hack-Regular.txt).
- Noto Emoji Regular: SIL Open Font License 1.1 — [full notice](docs/licenses/fonts/OFL.txt).
- emoji-icon-font: MIT — [full notice](docs/licenses/fonts/emoji-icon-font-mit-license.txt).

These notices cover the default fallback fonts as well as the primary canvas font. Font licensing is distinct from Rastera's application source-license decision, which remains pending. See the dependency packages for their software licenses.
