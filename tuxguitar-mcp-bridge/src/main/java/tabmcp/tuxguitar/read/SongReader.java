package tabmcp.tuxguitar.read;

import java.util.Iterator;

import app.tuxguitar.song.managers.TGSongManager;
import app.tuxguitar.song.models.TGChannel;
import app.tuxguitar.song.models.TGMeasureHeader;
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.song.models.TGString;
import app.tuxguitar.song.models.TGTrack;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/**
 * Maps the TuxGuitar model to the wire schema (see docs/PROTOCOL.md).
 * Must be called with the editor lock held.
 */
public class SongReader {

	public JsonObject readSong(TGSong song, TGSongManager songManager) {
		JsonObject result = new JsonObject();
		result.add("metadata", this.metadata(song));
		result.add("tracks", this.tracks(song, songManager));
		result.add("headers", this.headers(song));
		return result;
	}

	private JsonObject metadata(TGSong song) {
		JsonObject metadata = new JsonObject();
		metadata.addProperty("title", nonNull(song.getName()));
		metadata.addProperty("artist", nonNull(song.getArtist()));
		metadata.addProperty("album", nonNull(song.getAlbum()));
		metadata.addProperty("author", nonNull(song.getAuthor()));
		metadata.addProperty("comments", nonNull(song.getComments()));
		return metadata;
	}

	private JsonArray tracks(TGSong song, TGSongManager songManager) {
		JsonArray tracks = new JsonArray();
		for (int i = 0; i < song.countTracks(); i++) {
			TGTrack track = song.getTrack(i);
			JsonObject dto = new JsonObject();
			dto.addProperty("number", track.getNumber());
			dto.addProperty("name", nonNull(track.getName()));
			dto.add("strings", this.strings(track));
			TGChannel channel = songManager.getChannel(song, track.getChannelId());
			dto.addProperty("program", channel != null ? channel.getProgram() : 0);
			dto.addProperty("isPercussion", track.isPercussion());
			dto.addProperty("offset", track.getOffset());
			dto.addProperty("maxFret", track.getMaxFret());
			dto.addProperty("measureCount", track.countMeasures());
			tracks.add(dto);
		}
		return tracks;
	}

	private JsonArray strings(TGTrack track) {
		JsonArray strings = new JsonArray();
		for (TGString string : track.getStrings()) {
			JsonObject dto = new JsonObject();
			dto.addProperty("number", string.getNumber());
			dto.addProperty("openPitch", string.getValue());
			strings.add(dto);
		}
		return strings;
	}

	private JsonArray headers(TGSong song) {
		JsonArray headers = new JsonArray();
		Iterator<TGMeasureHeader> it = song.getMeasureHeaders();
		while (it.hasNext()) {
			TGMeasureHeader header = it.next();
			JsonObject dto = new JsonObject();
			dto.addProperty("number", header.getNumber());
			dto.addProperty("startTick", header.getStart());

			JsonObject timeSignature = new JsonObject();
			timeSignature.addProperty("numerator", header.getTimeSignature().getNumerator());
			timeSignature.addProperty("denominator", header.getTimeSignature().getDenominator().getValue());
			dto.add("timeSignature", timeSignature);

			dto.addProperty("tempoBpm", header.getTempo().getQuarterValue());
			dto.addProperty("repeatOpen", header.isRepeatOpen());
			dto.addProperty("repeatClose", Math.max(0, header.getRepeatClose()));
			dto.addProperty("repeatAlternative", Math.max(0, header.getRepeatAlternative()));
			if (header.getMarker() != null) {
				dto.addProperty("marker", header.getMarker().getTitle());
			}
			headers.add(dto);
		}
		return headers;
	}

	private static String nonNull(String value) {
		return value != null ? value : "";
	}
}
