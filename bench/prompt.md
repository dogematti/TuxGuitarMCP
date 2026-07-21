# TabBench canonical brief (v2)

Check the style guide for **metalcore**, then write me a full metalcore
song following its recipe, using the composition engines:

1. Create a **7-string A standard** track (use the preset), around 140
   BPM. Markers: Intro, Verse, Breakdown, Outro - about 20 bars total.
2. Seed the verse with `tuxguitar_generate_riff` - pick the scale from
   the recipe, and set the accent offsets to your planned kick pattern so
   guitar and kick lock from birth. Then develop it: the second half of
   the verse must be a *variation* (`vary_riff` or `evolve_riff`), never
   a copy.
3. Gate your main riff through `tuxguitar_hook_check` - revise until it
   PASSES.
4. The breakdown is the centerpiece: halftime, rhythm-only chugs on the
   open low A, and generate its drums with `tuxguitar_generate_interlock`
   so the kick lands in perfect unison with the guitar accents. A tempo
   drop is allowed if it serves the song.
5. Generate bass (accent-following, mind the register so stems stay
   audible). If your riff leaves gaps, add a `generate_counterline`
   answer line in the Verse or Outro.
6. Add transitions before your section boundaries
   (`tuxguitar_generate_transitions`) and run an ornament pass
   (`tuxguitar_ornament`) so the parts read like playing, not MIDI.
7. Refine with the AI Ear until the scorecard is clean:
   `tuxguitar_evaluate` with `style="metalcore"`, trust the per-section
   table over whole-song flags, watch the HUMAN-FEEL line and the
   DEVELOPMENT QUOTA, listen to the stems, and tell me the loudest and
   quietest measure of the final render.
8. Finish with `tuxguitar_track_themes` to prove the song remembers its
   own material, play it, then give me the pass summaries and the Cmd+Z
   rollback map.

Save a copy as **SAVE_NAME.tg** when done.

Continue autonomously until the song is finished and saved. Do not stop
to summarize or ask questions mid-way; report everything at the end.
