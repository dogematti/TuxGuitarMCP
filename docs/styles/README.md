# Player notes: your own style knowledge

Drop markdown files in `~/.tuxguitar-mcp/styles/` and every AI client
composing through TabMCP reads them automatically:

- `<style>.md` matching a built-in style name (spaces as hyphens, e.g.
  `death-metal.md`) is appended to that style's rubric under
  "PLAYER NOTES", with instructions that your notes override the generic
  recipe when they conflict.
- A file with a NEW name (e.g. `suomi-metal.md`) defines a custom style:
  `style_guide { style: "suomi metal" }` serves it standalone.
- Blends serve the dominant style's notes.

Write whatever a bandmate would need: fretboard vocabulary per tuning,
favorite positions, riff shapes, house rules ("no solos over
breakdowns"), tone notes. Freeform markdown - there is no schema.

`metalcore.example.md` in this directory is a starter template; copy it
to `~/.tuxguitar-mcp/styles/metalcore.md` and edit.
