package tabmcp.tuxguitar.edit;

import app.tuxguitar.document.TGDocumentManager;
import app.tuxguitar.editor.undo.TGUndoableManager;
import app.tuxguitar.editor.undo.impl.measure.TGUndoableMeasureGeneric;
import app.tuxguitar.song.factory.TGFactory;
import app.tuxguitar.song.managers.TGMeasureManager;
import app.tuxguitar.song.managers.TGSongManager;
import app.tuxguitar.song.models.TGDuration;
import app.tuxguitar.song.models.TGMeasure;
import app.tuxguitar.song.models.TGNote;
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.song.models.TGTrack;
import app.tuxguitar.song.models.TGVelocities;
import app.tuxguitar.util.TGContext;

/**
 * Milestone-1 spike: one hard-coded, fully undoable edit that proves the
 * bridge can mutate the score through TuxGuitar's undo system.
 *
 * Behavior: at track 1, measure 1, beat 1 — if a note exists on string 6 it
 * toggles between frets 5 and 7, otherwise a fret-5 quarter note is added.
 * A single Ctrl+Z in TuxGuitar must revert it completely.
 *
 * Must be called on the UI thread with the editor lock held.
 */
public class SpikeEdit {

	public static class Outcome {
		public final int track;
		public final int measure;
		public final String description;

		public Outcome(int track, int measure, String description) {
			this.track = track;
			this.measure = measure;
			this.description = description;
		}
	}

	public static Outcome run(TGContext context) {
		TGDocumentManager documentManager = TGDocumentManager.getInstance(context);
		TGSong song = documentManager.getSong();
		if (song == null || song.countTracks() == 0) {
			return null;
		}
		TGSongManager songManager = documentManager.getSongManager();
		TGTrack track = song.getTrack(0);
		TGMeasure measure = track.getMeasure(0);

		// Snapshot before, mutate, snapshot after, push to the undo stack —
		// the same sequence TGUndoableActionListener performs for built-in
		// actions mapped to UNDOABLE_MEASURE_GENERIC.
		TGUndoableMeasureGeneric undoable = TGUndoableMeasureGeneric.startUndo(context, measure);

		TGMeasureManager measureManager = songManager.getMeasureManager();
		long start = measure.getStart();
		String description;

		TGNote existing = measureManager.getNote(measure, start, 6);
		if (existing != null) {
			int newFret = existing.getValue() == 5 ? 7 : 5;
			description = "changed string 6 fret " + existing.getValue() + " -> " + newFret;
			existing.setValue(newFret);
		} else {
			TGFactory factory = songManager.getFactory();
			TGNote note = factory.newNote();
			note.setString(6);
			note.setValue(5);
			note.setVelocity(TGVelocities.DEFAULT);
			TGDuration duration = factory.newDuration();
			duration.setValue(TGDuration.QUARTER);
			measureManager.addNote(measure, start, note, duration, 0);
			description = "added quarter note: string 6, fret 5";
		}

		TGUndoableManager.getInstance(context).addEdit(undoable.endUndo(measure));

		return new Outcome(track.getNumber(), measure.getNumber(), description);
	}
}
