# diapo

**Slides**: presentations for Linux in Rust ([egui](https://github.com/emilk/egui)).
It opens and saves PowerPoint files (`.pptx`).

- Slide list down the left (duplicate, reorder, delete), a 16:9 canvas.
- Text boxes, rectangles, ellipses, lines and pictures: draw, move,
  resize (pictures keep their shape), arrange front and back, duplicate.
- Text size, font, bold / italic / underline, colour, alignment, bullet
  points; fills and outlines; slide backgrounds.
- New presentations start black with white text.
- Presenting: F5 (or Present), fullscreen; arrows, space or a click to go
  on, Escape to stop.
- PowerPoint files from elsewhere open as PowerPoint shows them:
  placeholders placed by their layout and master, theme colours, groups,
  pictures, 4:3 decks centred. Saved files are plain, valid `.pptx` with
  each element's own colours, so they look the same in PowerPoint,
  Keynote and Google Slides.
- **Claude** (Ctrl+J): Claude reads the slides and adds, changes,
  reorders and removes slides and elements through the app's own tools,
  live; every change undoes with Ctrl+Z. Needs
  [Claude Code](https://claude.com/claude-code) (`claude`) installed and
  logged in.

The look, title bar and Claude panel come from
[atelier](https://github.com/GoldenSheeeep/atelier), shared with
[tableur](https://github.com/GoldenSheeeep/tableur) (Sheets) and
[plume](https://github.com/GoldenSheeeep/plume) (Docs).

## Building

```sh
cargo build --release   # target/release/slides
```

At run time it needs `libGL`, `wayland` and `libxkbcommon`, and
`fc-match` (fontconfig) for fonts.

## Not yet

One style per text box (not per word), speaker notes, transitions and
animations; charts and SmartArt from other files are left out, tables
come in as text.

## License

MIT
