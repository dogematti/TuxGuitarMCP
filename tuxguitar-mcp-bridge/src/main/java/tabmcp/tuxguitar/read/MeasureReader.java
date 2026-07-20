package tabmcp.tuxguitar.read;

import java.util.Iterator;

import app.tuxguitar.song.models.TGBeat;
import app.tuxguitar.song.models.TGDuration;
import app.tuxguitar.song.models.TGMeasure;
import app.tuxguitar.song.models.TGNote;
import app.tuxguitar.song.models.TGNoteEffect;
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.song.models.TGTrack;
import app.tuxguitar.song.models.TGVoice;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Maps measure content (beats/voices/notes/effects) to the wire schema.
 * Must be called with the editor lock held.
 */
public class MeasureReader {

	/** Returns null when the track does not exist. */
	public TGTrack findTrack(TGSong song, int trackNumber) {
		Iterator<TGTrack> it = song.getTracks();
		while (it.hasNext()) {
			TGTrack track = it.next();
			if (track.getNumber() == trackNumber) {
				return track;
			}
		}
		return null;
	}

	public JsonArray readMeasures(TGTrack track, int fromMeasure, int toMeasure) {
		JsonArray measures = new JsonArray();
		for (int number = fromMeasure; number <= toMeasure; number++) {
			measures.add(this.measure(track.getMeasure(number - 1)));
		}
		return measures;
	}

	private JsonObject measure(TGMeasure measure) {
		JsonObject dto = new JsonObject();
		dto.addProperty("number", measure.getNumber());
		dto.addProperty("startTick", measure.getStart());
		dto.addProperty("keySignature", measure.getKeySignature());
		JsonArray beats = new JsonArray();
		for (TGBeat beat : measure.getBeats()) {
			beats.add(this.beat(beat));
		}
		dto.add("beats", beats);
		return dto;
	}

	private JsonObject beat(TGBeat beat) {
		JsonObject dto = new JsonObject();
		dto.addProperty("startTick", beat.getStart());
		JsonArray voices = new JsonArray();
		for (int i = 0; i < TGBeat.MAX_VOICES; i++) {
			TGVoice voice = beat.getVoice(i);
			if (voice != null && !voice.isEmpty()) {
				voices.add(this.voice(voice));
			}
		}
		dto.add("voices", voices);
		return dto;
	}

	private JsonObject voice(TGVoice voice) {
		JsonObject dto = new JsonObject();
		dto.addProperty("index", voice.getIndex());
		dto.add("duration", this.duration(voice.getDuration()));
		if (voice.getNotes().isEmpty()) {
			dto.addProperty("isRest", true);
		}
		JsonArray notes = new JsonArray();
		for (TGNote note : voice.getNotes()) {
			notes.add(this.note(note));
		}
		dto.add("notes", notes);
		return dto;
	}

	private JsonObject duration(TGDuration duration) {
		JsonObject dto = new JsonObject();
		dto.addProperty("value", duration.getValue());
		if (duration.isDotted()) {
			dto.addProperty("dotted", true);
		}
		if (duration.isDoubleDotted()) {
			dto.addProperty("doubleDotted", true);
		}
		JsonObject tuplet = new JsonObject();
		tuplet.addProperty("enters", duration.getDivision().getEnters());
		tuplet.addProperty("times", duration.getDivision().getTimes());
		dto.add("tuplet", tuplet);
		return dto;
	}

	private JsonObject note(TGNote note) {
		JsonObject dto = new JsonObject();
		dto.addProperty("string", note.getString());
		dto.addProperty("fret", note.getValue());
		dto.addProperty("velocity", note.getVelocity());
		if (note.isTiedNote()) {
			dto.addProperty("tied", true);
		}
		dto.add("effects", this.effects(note.getEffect()));
		return dto;
	}

	private JsonObject effects(TGNoteEffect effect) {
		JsonObject dto = new JsonObject();
		if (effect == null) {
			return dto;
		}
		addFlag(dto, "vibrato", effect.isVibrato());
		addFlag(dto, "deadNote", effect.isDeadNote());
		addFlag(dto, "slide", effect.isSlide());
		addFlag(dto, "hammer", effect.isHammer());
		addFlag(dto, "ghostNote", effect.isGhostNote());
		addFlag(dto, "accent", effect.isAccentuatedNote());
		addFlag(dto, "heavyAccent", effect.isHeavyAccentuatedNote());
		addFlag(dto, "palmMute", effect.isPalmMute());
		addFlag(dto, "staccato", effect.isStaccato());
		addFlag(dto, "letRing", effect.isLetRing());
		addFlag(dto, "tapping", effect.isTapping());
		addFlag(dto, "slapping", effect.isSlapping());
		addFlag(dto, "popping", effect.isPopping());
		addFlag(dto, "fadeIn", effect.isFadeIn());
		if (effect.isBend() && effect.getBend() != null) {
			JsonObject bend = new JsonObject();
			JsonArray points = new JsonArray();
			for (app.tuxguitar.song.models.effects.TGEffectBend.BendPoint point
					: effect.getBend().getPoints()) {
				JsonObject dtoPoint = new JsonObject();
				dtoPoint.addProperty("position", point.getPosition());
				dtoPoint.addProperty("value", point.getValue());
				points.add(dtoPoint);
			}
			bend.add("points", points);
			dto.add("bend", bend);
		}
		addFlag(dto, "tremoloBar", effect.isTremoloBar());
		if (effect.isHarmonic() && effect.getHarmonic() != null) {
			JsonObject harmonic = new JsonObject();
			harmonic.addProperty("type", harmonicName(effect.getHarmonic().getType()));
			if (effect.getHarmonic().getData() != 0) {
				harmonic.addProperty("data", effect.getHarmonic().getData());
			}
			dto.add("harmonic", harmonic);
		}
		if (effect.isGrace() && effect.getGrace() != null) {
			JsonObject grace = new JsonObject();
			grace.addProperty("fret", effect.getGrace().getFret());
			grace.addProperty("duration", effect.getGrace().getDuration());
			if (effect.getGrace().isOnBeat()) {
				grace.addProperty("onBeat", true);
			}
			if (effect.getGrace().isDead()) {
				grace.addProperty("dead", true);
			}
			grace.addProperty("transition", graceTransitionName(effect.getGrace().getTransition()));
			dto.add("grace", grace);
		}
		if (effect.isTrill() && effect.getTrill() != null) {
			JsonObject trill = new JsonObject();
			trill.addProperty("fret", effect.getTrill().getFret());
			trill.addProperty("speed", effect.getTrill().getDuration().getValue());
			dto.add("trill", trill);
		}
		if (effect.isTremoloPicking() && effect.getTremoloPicking() != null) {
			JsonObject tremolo = new JsonObject();
			tremolo.addProperty("speed", effect.getTremoloPicking().getDuration().getValue());
			dto.add("tremoloPicking", tremolo);
		}
		return dto;
	}

	private static String graceTransitionName(int transition) {
		switch (transition) {
			case app.tuxguitar.song.models.effects.TGEffectGrace.TRANSITION_SLIDE:
				return "slide";
			case app.tuxguitar.song.models.effects.TGEffectGrace.TRANSITION_BEND:
				return "bend";
			case app.tuxguitar.song.models.effects.TGEffectGrace.TRANSITION_HAMMER:
				return "hammer";
			default:
				return "none";
		}
	}

	private static void addFlag(JsonObject dto, String name, boolean value) {
		if (value) {
			dto.addProperty(name, true);
		}
	}

	/** Maps TGEffectHarmonic.TYPE_* to the wire names. */
	public static String harmonicName(int type) {
		switch (type) {
			case app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_NATURAL: return "natural";
			case app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_ARTIFICIAL: return "artificial";
			case app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_TAPPED: return "tapped";
			case app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_PINCH: return "pinch";
			case app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_SEMI: return "semi";
			default: return "natural";
		}
	}

	/** Maps wire names back to TGEffectHarmonic.TYPE_*. */
	public static int harmonicType(String name) {
		if ("artificial".equals(name)) {
			return app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_ARTIFICIAL;
		}
		if ("tapped".equals(name)) {
			return app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_TAPPED;
		}
		if ("pinch".equals(name)) {
			return app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_PINCH;
		}
		if ("semi".equals(name)) {
			return app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_SEMI;
		}
		return app.tuxguitar.song.models.effects.TGEffectHarmonic.TYPE_NATURAL;
	}
}
