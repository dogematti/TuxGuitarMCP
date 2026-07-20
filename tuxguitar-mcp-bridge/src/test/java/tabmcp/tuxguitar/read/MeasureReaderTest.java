package tabmcp.tuxguitar.read;

import app.tuxguitar.song.factory.TGFactory;
import app.tuxguitar.song.models.TGBeat;
import app.tuxguitar.song.models.TGDuration;
import app.tuxguitar.song.models.TGMeasure;
import app.tuxguitar.song.models.TGMeasureHeader;
import app.tuxguitar.song.models.TGNote;
import app.tuxguitar.song.models.TGTrack;
import app.tuxguitar.song.models.effects.TGEffectHarmonic;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import org.junit.Test;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

public class MeasureReaderTest {

	@Test
	public void harmonicNamesRoundTripAllTypes() {
		int[] types = {
			TGEffectHarmonic.TYPE_NATURAL, TGEffectHarmonic.TYPE_ARTIFICIAL,
			TGEffectHarmonic.TYPE_TAPPED, TGEffectHarmonic.TYPE_PINCH,
			TGEffectHarmonic.TYPE_SEMI,
		};
		for (int type : types) {
			assertEquals(type, MeasureReader.harmonicType(MeasureReader.harmonicName(type)));
		}
		assertEquals("pinch", MeasureReader.harmonicName(TGEffectHarmonic.TYPE_PINCH));
		assertEquals(TGEffectHarmonic.TYPE_NATURAL, MeasureReader.harmonicType("garbage"));
	}

	@Test
	public void measureDtoCarriesStartTickNotesAndPinchHarmonic() {
		TGFactory factory = new TGFactory();

		TGMeasureHeader header = factory.newHeader();
		header.setNumber(1);
		header.setStart(TGDuration.QUARTER_TIME);

		TGTrack track = factory.newTrack();
		track.setNumber(1);
		TGMeasure measure = factory.newMeasure(header);
		track.addMeasure(measure);

		TGBeat beat = factory.newBeat();
		beat.setStart(TGDuration.QUARTER_TIME);
		beat.setPreciseStart(TGDuration.toPreciseTime(TGDuration.QUARTER_TIME));
		measure.addBeat(beat);

		TGNote note = factory.newNote();
		note.setString(6);
		note.setValue(3);
		note.setVelocity(95);
		TGEffectHarmonic harmonic = factory.newEffectHarmonic();
		harmonic.setType(TGEffectHarmonic.TYPE_PINCH);
		note.getEffect().setHarmonic(harmonic);
		beat.getVoice(0).getDuration().setValue(TGDuration.EIGHTH);
		beat.getVoice(0).addNote(note);

		JsonArray measures = new MeasureReader().readMeasures(track, 1, 1);
		assertEquals(1, measures.size());
		JsonObject dto = measures.get(0).getAsJsonObject();
		assertEquals(TGDuration.QUARTER_TIME, dto.get("startTick").getAsLong());

		JsonObject beatDto = dto.getAsJsonArray("beats").get(0).getAsJsonObject();
		JsonObject noteDto = beatDto.getAsJsonArray("voices").get(0).getAsJsonObject()
			.getAsJsonArray("notes").get(0).getAsJsonObject();
		assertEquals(6, noteDto.get("string").getAsInt());
		assertEquals(3, noteDto.get("fret").getAsInt());

		JsonObject effects = noteDto.getAsJsonObject("effects");
		assertTrue("harmonic must serialize as an object", effects.get("harmonic").isJsonObject());
		assertEquals("pinch", effects.getAsJsonObject("harmonic").get("type").getAsString());
	}
}
