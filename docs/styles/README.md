# Player notes: your own style knowledge

Drop markdown files in `~/.tuxguitar-mcp/styles/` and every AI client
composing through TabMCP reads them automatically:

- `<style>.md` matching a built-in style name (spaces as hyphens, e.g.
  `death-metal.md`) is appended to that style's rubric under
  "PLAYER NOTES", with instructions that your notes override the generic
  recipe when they conflict.
- `<style>.<tuning>.md` is TUNING-SPECIFIC: served only when the score
  (or an explicit `tuning` argument) matches that tuning preset, layered
  after the base notes as the most-specific layer. The tuning part is a
  prefix of the preset's slug, so `metalcore.7-string-a.md` matches
  "7-string A standard". The tuning is auto-detected from the open
  score's first melodic track; pass `tuning: "7-string A standard"` to
  force it.
- A file with a NEW name (e.g. `suomi-metal.md`, with or without a
  tuning part) defines a custom style: `style_guide { style: "suomi
  metal" }` serves it standalone.
- Blends serve the dominant style's notes.

Precedence when layers conflict: tuning-specific notes > base player
notes > built-in rubric.

Write whatever a bandmate would need: fretboard vocabulary per tuning,
favorite positions, riff shapes, house rules ("no solos over
breakdowns"), tone notes. Freeform markdown - there is no schema, and
files are read fresh on every call (no rebuild, no restart).

`metalcore.example.md` in this directory is a starter template; copy it
to e.g. `~/.tuxguitar-mcp/styles/metalcore.7-string-a-standard.md` and
edit.
