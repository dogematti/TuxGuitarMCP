package tabmcp.tuxguitar.edit;

import app.tuxguitar.document.TGDocumentManager;
import app.tuxguitar.editor.undo.TGUndoableManager;
import app.tuxguitar.editor.undo.impl.measure.TGUndoableAddMeasure;
import app.tuxguitar.editor.undo.impl.measure.TGUndoableMeasureGeneric;
import app.tuxguitar.song.factory.TGFactory;
import app.tuxguitar.song.managers.TGMeasureManager;
import app.tuxguitar.song.managers.TGSongManager;
import app.tuxguitar.song.models.TGDuration;
import app.tuxguitar.song.models.TGMeasure;
import app.tuxguitar.song.models.TGNote;
import app.tuxguitar.song.models.TGNoteEffect;
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.song.models.TGTrack;
import app.tuxguitar.song.models.TGVelocities;
import app.tuxguitar.util.TGContext;
import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import tabmcp.tuxguitar.read.MeasureReader;
import tabmcp.tuxguitar.rpc.RpcException;

/**
 * Applies a replaceMeasureRange change: swaps the contents of a measure
 * range on one track, appending measures to the song when the range extends
 * past its end. One undoable edit on the undo stack per change-set.
 *
 * Must be called on the UI thread with the editor lock held.
 */
public class ChangesetApplier {

	public static class Outcome {
		public int measuresReplaced;
		public int measuresAdded;
		public int notesBefore;
		public int notesAfter;
	}

	public Outcome applyReplaceMeasureRange(TGContext context, JsonObject change) throws RpcException {
		int trackNumber = change.has("trackNumber") ? change.get("trackNumber").getAsInt() : 1;
		int fromMeasure = change.has("fromMeasure") ? change.get("fromMeasure").getAsInt() : 1;
		JsonArray measures = change.getAsJsonArray("measures");
		if (measures == null || measures.size() == 0) {
			throw new RpcException(RpcException.INVALID_RANGE, "changes[0].measures is empty");
		}

		TGDocumentManager documentManager = TGDocumentManager.getInstance(context);
		TGSong song = documentManager.getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		TGSongManager songManager = documentManager.getSongManager();
		TGTrack track = new MeasureReader().findTrack(song, trackNumber);
		if (track == null || fromMeasure < 1) {
			throw new RpcException(RpcException.INVALID_RANGE,
				"invalid track " + trackNumber + " or fromMeasure " + fromMeasure);
		}

		int toMeasure = fromMeasure + measures.size() - 1;
		int existingCount = track.countMeasures();
		if (fromMeasure > existingCount + 1) {
			throw new RpcException(RpcException.INVALID_RANGE,
				"fromMeasure " + fromMeasure + " would leave a gap: the song has "
					+ existingCount + " measures");
		}

		ReversibleComposite composite = new ReversibleComposite();
		Outcome outcome = new Outcome();

		// 1. Append measures if the range extends past the end of the song.
		for (int number = existingCount + 1; number <= toMeasure; number++) {
			composite.addEdit(TGUndoableAddMeasure.startUndo(context, number));
			songManager.addNewMeasure(song, number);
			outcome.measuresAdded++;
		}

		// 2. Replace content measure by measure (snapshot before/after).
		TGMeasureManager measureManager = songManager.getMeasureManager();
		TGFactory factory = songManager.getFactory();
		for (int i = 0; i < measures.size(); i++) {
			int number = fromMeasure + i;
			TGMeasure measure = track.getMeasure(number - 1);
			TGUndoableMeasureGeneric undoable = TGUndoableMeasureGeneric.startUndo(context, measure);

			outcome.notesBefore += countNotes(measure);
			this.writeMeasure(measure, measures.get(i).getAsJsonObject(), measureManager, factory);
			measureManager.autoCompleteSilences(measure);
			outcome.notesAfter += countNotes(measure);
			outcome.measuresReplaced++;

			composite.addEdit(undoable.endUndo(measure));
		}

		TGUndoableManager.getInstance(context).addEdit(composite);
		return outcome;
	}

	private void writeMeasure(TGMeasure measure, JsonObject wire, TGMeasureManager measureManager,
			TGFactory factory) {
		while (measure.countBeats() > 0) {
			measure.removeBeat(measure.getBeat(0));
		}
		long actualStart = measure.getStart();
		long wireStart = wire.has("startTick") ? wire.get("startTick").getAsLong() : 0L;

		JsonArray beats = wire.getAsJsonArray("beats");
		if (beats == null) {
			return;
		}
		// Build beats directly on the model, the same way TuxGuitar's own
		// .tg reader does (TGSongReaderImpl): TGMeasureManager.addNote needs
		// a pre-existing beat at the position, which a cleared measure lacks.
		for (JsonElement beatElement : beats) {
			JsonObject wireBeat = beatElement.getAsJsonObject();
			long beatTick = wireBeat.has("startTick") ? wireBeat.get("startTick").getAsLong() : wireStart;
			long start = actualStart + Math.max(0, beatTick - wireStart);

			JsonArray voices = wireBeat.getAsJsonArray("voices");
			if (voices == null) {
				continue;
			}
			app.tuxguitar.song.models.TGBeat beat = factory.newBeat();
			// setStart(long) nulls preciseStart, which 2.x layout code
			// requires — restore it explicitly (see TGBeat.setStart javadoc).
			beat.setStart(start);
			beat.setPreciseStart(TGDuration.toPreciseTime(start));
			measure.addBeat(beat);

			for (JsonElement voiceElement : voices) {
				JsonObject wireVoice = voiceElement.getAsJsonObject();
				int voiceIndex = Math.min(
					Math.max(wireVoice.has("index") ? wireVoice.get("index").getAsInt() : 0, 0),
					app.tuxguitar.song.models.TGBeat.MAX_VOICES - 1);
				TGDuration duration = this.duration(wireVoice.getAsJsonObject("duration"), factory);

				app.tuxguitar.song.models.TGVoice voice = beat.getVoice(voiceIndex);
				voice.getDuration().copyFrom(duration);
				// An explicit rest voice is non-empty with zero notes; a voice
				// with notes becomes non-empty via addNote below; anything
				// else stays empty and autoCompleteSilences normalizes it.
				boolean isRest = wireVoice.has("isRest") && wireVoice.get("isRest").getAsBoolean();
				voice.setEmpty(!isRest);

				JsonArray notes = wireVoice.getAsJsonArray("notes");
				if (notes == null) {
					continue;
				}
				for (JsonElement noteElement : notes) {
					voice.addNote(this.note(noteElement.getAsJsonObject(), factory));
				}
			}
		}
	}

	private TGDuration duration(JsonObject wire, TGFactory factory) {
		TGDuration duration = factory.newDuration();
		if (wire == null) {
			return duration;
		}
		if (wire.has("value")) {
			duration.setValue(wire.get("value").getAsInt());
		}
		duration.setDotted(wire.has("dotted") && wire.get("dotted").getAsBoolean());
		duration.setDoubleDotted(wire.has("doubleDotted") && wire.get("doubleDotted").getAsBoolean());
		JsonObject tuplet = wire.getAsJsonObject("tuplet");
		if (tuplet != null && tuplet.has("enters") && tuplet.has("times")) {
			duration.getDivision().setEnters(tuplet.get("enters").getAsInt());
			duration.getDivision().setTimes(tuplet.get("times").getAsInt());
		}
		return duration;
	}

	private TGNote note(JsonObject wire, TGFactory factory) {
		TGNote note = factory.newNote();
		note.setString(wire.has("string") ? wire.get("string").getAsInt() : 1);
		note.setValue(wire.has("fret") ? wire.get("fret").getAsInt() : 0);
		note.setVelocity(wire.has("velocity") ? wire.get("velocity").getAsInt() : TGVelocities.DEFAULT);
		note.setTiedNote(wire.has("tied") && wire.get("tied").getAsBoolean());
		note.setEffect(this.effects(wire.getAsJsonObject("effects"), factory));
		return note;
	}

	private TGNoteEffect effects(JsonObject wire, TGFactory factory) {
		TGNoteEffect effect = factory.newEffect();
		if (wire == null) {
			return effect;
		}
		effect.setVibrato(flag(wire, "vibrato"));
		effect.setDeadNote(flag(wire, "deadNote"));
		effect.setSlide(flag(wire, "slide"));
		effect.setHammer(flag(wire, "hammer"));
		effect.setGhostNote(flag(wire, "ghostNote"));
		effect.setAccentuatedNote(flag(wire, "accent"));
		effect.setHeavyAccentuatedNote(flag(wire, "heavyAccent"));
		effect.setPalmMute(flag(wire, "palmMute"));
		effect.setStaccato(flag(wire, "staccato"));
		effect.setLetRing(flag(wire, "letRing"));
		effect.setTapping(flag(wire, "tapping"));
		effect.setSlapping(flag(wire, "slapping"));
		effect.setPopping(flag(wire, "popping"));
		effect.setFadeIn(flag(wire, "fadeIn"));

		JsonElement harmonic = wire.get("harmonic");
		if (harmonic != null && !(harmonic.isJsonPrimitive()
				&& harmonic.getAsJsonPrimitive().isBoolean() && !harmonic.getAsBoolean())) {
			app.tuxguitar.song.models.effects.TGEffectHarmonic tgHarmonic = factory.newEffectHarmonic();
			if (harmonic.isJsonObject()) {
				JsonObject h = harmonic.getAsJsonObject();
				tgHarmonic.setType(tabmcp.tuxguitar.read.MeasureReader.harmonicType(
					h.has("type") ? h.get("type").getAsString() : "natural"));
				tgHarmonic.setData(h.has("data") ? h.get("data").getAsInt() : 0);
			} else {
				tgHarmonic.setType(
					app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_NATURAL);
			}
			effect.setHarmonic(tgHarmonic);
		}

		JsonElement bend = wire.get("bend");
		if (bend != null && !(bend.isJsonPrimitive()
				&& bend.getAsJsonPrimitive().isBoolean() && !bend.getAsBoolean())) {
			app.tuxguitar.song.models.effects.TGEffectBend tgBend = factory.newEffectBend();
			com.google.gson.JsonArray points = (bend.isJsonObject()
				&& bend.getAsJsonObject().has("points"))
					? bend.getAsJsonObject().getAsJsonArray("points") : null;
			if (points != null && points.size() > 0) {
				for (JsonElement pointElement : points) {
					JsonObject point = pointElement.getAsJsonObject();
					tgBend.addPoint(
						point.has("position") ? point.get("position").getAsInt() : 0,
						point.has("value") ? point.get("value").getAsInt() : 0);
				}
			} else {
				// Standard full-tone bend: up over the first half, hold.
				tgBend.addPoint(0, 0);
				tgBend.addPoint(6, 2);
				tgBend.addPoint(12, 2);
			}
			effect.setBend(tgBend);
		}

		// Remaining complex effects (grace, trill, tremolo picking/bar) still
		// arrive as presence flags and are not applied yet.
		return effect;
	}

	private static boolean flag(JsonObject wire, String name) {
		return wire.has(name) && wire.get(name).getAsBoolean();
	}

	private static int countNotes(TGMeasure measure) {
		int count = 0;
		for (app.tuxguitar.song.models.TGBeat beat : measure.getBeats()) {
			for (int v = 0; v < app.tuxguitar.song.models.TGBeat.MAX_VOICES; v++) {
				app.tuxguitar.song.models.TGVoice voice = beat.getVoice(v);
				if (voice != null && !voice.isEmpty()) {
					count += voice.getNotes().size();
				}
			}
		}
		return count;
	}
}
