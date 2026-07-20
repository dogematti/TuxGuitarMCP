package tabmcp.tuxguitar.rpc;

import java.util.Collections;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import app.tuxguitar.action.TGActionContext;
import app.tuxguitar.action.TGActionManager;
import app.tuxguitar.app.TuxGuitar;
import app.tuxguitar.document.TGDocumentManager;
import app.tuxguitar.editor.TGEditorManager;
import app.tuxguitar.editor.undo.TGUndoableManager;
import app.tuxguitar.app.view.component.tab.Caret;
import app.tuxguitar.app.view.component.tab.Selector;
import app.tuxguitar.app.view.component.tab.Tablature;
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.song.models.TGTrack;
import app.tuxguitar.util.TGContext;
import app.tuxguitar.util.TGSynchronizer;
import app.tuxguitar.util.TGVersion;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import tabmcp.tuxguitar.edit.ChangesetApplier;
import tabmcp.tuxguitar.edit.SpikeEdit;
import tabmcp.tuxguitar.read.MeasureReader;
import tabmcp.tuxguitar.read.RevisionTracker;
import tabmcp.tuxguitar.read.SongReader;

/**
 * Implements the bridge methods (everything after authentication).
 * Called from the socket thread; anything touching the score model runs
 * under the editor lock, and anything mutating runs on the UI thread.
 */
public class BridgeService {

	public static final int PROTOCOL_VERSION = 1;
	public static final String PLUGIN_VERSION = "0.4.2";

	private static final long EDIT_TIMEOUT_SECONDS = 10;

	private final TGContext context;
	private final RevisionTracker revisionTracker;
	private final SongReader songReader;
	private final MeasureReader measureReader;

	public BridgeService(TGContext context, RevisionTracker revisionTracker) {
		this.context = context;
		this.revisionTracker = revisionTracker;
		this.songReader = new SongReader();
		this.measureReader = new MeasureReader();
	}

	public RevisionTracker getRevisionTracker() {
		return this.revisionTracker;
	}

	public JsonObject helloResult() {
		JsonObject serverInfo = new JsonObject();
		serverInfo.addProperty("tuxguitarVersion", TGVersion.CURRENT.getVersion());
		serverInfo.addProperty("pluginVersion", PLUGIN_VERSION);

		JsonArray capabilities = new JsonArray();
		capabilities.add("read");
		capabilities.add("selection");
		capabilities.add("edit");
		capabilities.add("write");
		capabilities.add("tracks");
		capabilities.add("playback");
		capabilities.add("undo");

		JsonObject result = new JsonObject();
		result.addProperty("protocolVersion", PROTOCOL_VERSION);
		result.add("serverInfo", serverInfo);
		result.add("capabilities", capabilities);
		return result;
	}

	public JsonObject ping() {
		JsonObject result = new JsonObject();
		result.addProperty("revision", this.revisionTracker.getRevision());
		result.addProperty("documentOpen", TGDocumentManager.getInstance(this.context).getSong() != null);
		result.addProperty("playing", TuxGuitar.getInstance().getPlayer().isRunning());
		return result;
	}

	public JsonObject readSong() throws RpcException {
		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		final AtomicReference<JsonObject> resultRef = new AtomicReference<>();
		TGEditorManager.getInstance(this.context).runLocked(new Runnable() {
			public void run() {
				TGSong song = documentManager.getSong();
				if (song != null) {
					resultRef.set(BridgeService.this.songReader.readSong(song, documentManager.getSongManager()));
				}
			}
		});
		JsonObject result = resultRef.get();
		if (result == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		result.addProperty("revision", this.revisionTracker.getRevision());
		result.addProperty("documentId", this.revisionTracker.getDocumentId());
		return result;
	}

	public JsonObject readMeasures(JsonObject params) throws RpcException {
		final int trackNumber = params.has("trackNumber") ? params.get("trackNumber").getAsInt() : 1;
		final int from = params.has("fromMeasure") ? params.get("fromMeasure").getAsInt() : 1;
		final int to = params.has("toMeasure") ? params.get("toMeasure").getAsInt() : from;

		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		final AtomicReference<JsonObject> resultRef = new AtomicReference<>();
		final AtomicReference<String> errorRef = new AtomicReference<>();
		TGEditorManager.getInstance(this.context).runLocked(new Runnable() {
			public void run() {
				TGSong song = documentManager.getSong();
				if (song == null) {
					errorRef.set(RpcException.NO_DOCUMENT);
					return;
				}
				TGTrack track = BridgeService.this.measureReader.findTrack(song, trackNumber);
				if (track == null || from < 1 || to < from || to > track.countMeasures()) {
					errorRef.set(RpcException.INVALID_RANGE);
					return;
				}
				JsonObject result = new JsonObject();
				result.addProperty("trackNumber", trackNumber);
				result.addProperty("fromMeasure", from);
				result.addProperty("toMeasure", to);
				result.add("measures", BridgeService.this.measureReader.readMeasures(track, from, to));
				resultRef.set(result);
			}
		});
		if (errorRef.get() != null) {
			if (RpcException.NO_DOCUMENT.equals(errorRef.get())) {
				throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
			}
			throw new RpcException(RpcException.INVALID_RANGE,
				"invalid track " + trackNumber + " or measure range " + from + "-" + to);
		}
		JsonObject result = resultRef.get();
		result.addProperty("revision", this.revisionTracker.getRevision());
		result.addProperty("documentId", this.revisionTracker.getDocumentId());
		return result;
	}

	public JsonObject readSelection() throws RpcException {
		final AtomicReference<JsonObject> resultRef = new AtomicReference<>();
		this.runOnUiThread(new Runnable() {
			public void run() {
				JsonObject result = new JsonObject();
				Tablature tablature = TuxGuitar.getInstance().getTablatureEditor().getTablature();
				Selector selector = tablature.getSelector();
				Caret caret = tablature.getCaret();

				boolean active = selector != null && selector.isActive()
					&& selector.getStartBeat() != null && selector.getEndBeat() != null;
				result.addProperty("active", active);
				if (active) {
					result.addProperty("trackNumber",
						caret.getTrack() != null ? caret.getTrack().getNumber() : 1);
					result.addProperty("fromMeasure", selector.getStartBeat().getMeasure().getNumber());
					result.addProperty("toMeasure", selector.getEndBeat().getMeasure().getNumber());
				}
				if (caret != null && caret.getTrack() != null && caret.getMeasure() != null) {
					JsonObject caretDto = new JsonObject();
					caretDto.addProperty("trackNumber", caret.getTrack().getNumber());
					caretDto.addProperty("measureNumber", caret.getMeasure().getNumber());
					caretDto.addProperty("tick", caret.getPosition());
					caretDto.addProperty("stringNumber", caret.getStringNumber());
					result.add("caret", caretDto);
				}
				resultRef.set(result);
			}
		});
		JsonObject result = resultRef.get();
		if (result == null) {
			throw new RpcException(RpcException.INTERNAL, "selection state unavailable");
		}
		result.addProperty("revision", this.revisionTracker.getRevision());
		return result;
	}

	public JsonObject applyChangeset(JsonObject params) throws RpcException {
		if (!params.has("expectedRevision")) {
			throw new RpcException(RpcException.STALE_REVISION,
				"apply_changeset requires expectedRevision");
		}
		final long expectedRevision = params.get("expectedRevision").getAsLong();
		com.google.gson.JsonArray changes = params.getAsJsonArray("changes");
		if (changes == null || changes.size() != 1) {
			throw new RpcException(RpcException.UNSUPPORTED,
				"protocol v1 supports exactly one change per change-set");
		}
		final JsonObject change = changes.get(0).getAsJsonObject();
		String type = change.has("type") ? change.get("type").getAsString() : "";
		if (!"replaceMeasureRange".equals(type)) {
			throw new RpcException(RpcException.UNSUPPORTED, "unsupported change type: " + type);
		}

		final AtomicReference<ChangesetApplier.Outcome> outcomeRef = new AtomicReference<>();
		final AtomicReference<RpcException> rpcErrorRef = new AtomicReference<>();
		this.runOnUiThread(new Runnable() {
			public void run() {
				final TGEditorManager editor = TGEditorManager.getInstance(BridgeService.this.context);
				editor.runLocked(new Runnable() {
					public void run() {
						long current = BridgeService.this.revisionTracker.getRevision();
						if (current != expectedRevision) {
							rpcErrorRef.set(new RpcException(RpcException.STALE_REVISION,
								"score changed: expected revision " + expectedRevision
									+ ", current is " + current + " — re-read and retry"));
							return;
						}
						try {
							outcomeRef.set(new ChangesetApplier()
								.applyReplaceMeasureRange(BridgeService.this.context, change));
						} catch (RpcException e) {
							rpcErrorRef.set(e);
						}
					}
				});
				if (outcomeRef.get() != null) {
					editor.updateSong();
					editor.redraw();
				}
			}
		});
		if (rpcErrorRef.get() != null) {
			throw rpcErrorRef.get();
		}
		ChangesetApplier.Outcome outcome = outcomeRef.get();
		if (outcome == null) {
			throw new RpcException(RpcException.EDIT_FAILED, "change-set was not applied");
		}
		JsonObject result = new JsonObject();
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		result.addProperty("measuresReplaced", outcome.measuresReplaced);
		result.addProperty("measuresAdded", outcome.measuresAdded);
		result.addProperty("notesBefore", outcome.notesBefore);
		result.addProperty("notesAfter", outcome.notesAfter);
		return result;
	}

	public JsonObject createTrack(JsonObject params) throws RpcException {
		final String name = params.has("name") ? params.get("name").getAsString() : null;
		final com.google.gson.JsonArray strings =
			params.has("strings") ? params.getAsJsonArray("strings") : null;

		// Actions are executed on THIS (socket) thread, the pattern TuxGuitar
		// itself uses (TGActionProcessor.process spawns worker threads):
		// TGLockableActionListener defers lockable actions when they are
		// invoked from the UI thread, which would race our result checks.
		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		TGSong song = documentManager.getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		int before = song.countTracks();

		// action.track.add-new is wired to TGUndoableAddTrackController
		// in the app's action config, so undo comes with it.
		app.tuxguitar.editor.action.TGActionProcessor add =
			new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.track.add-new");
		add.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_SONG, song);
		add.processOnCurrentThread();

		if (song.countTracks() != before + 1) {
			throw new RpcException(RpcException.EDIT_FAILED, "track was not created");
		}
		TGTrack track = song.getTrack(song.countTracks() - 1);

		if (name != null) {
			app.tuxguitar.editor.action.TGActionProcessor rename =
				new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.track.set-name");
			rename.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_TRACK, track);
			rename.setAttribute("name", name);
			rename.processOnCurrentThread();
		}
		if (strings != null && strings.size() > 0) {
			this.runChangeTuning(track, strings);
		}

		// Normalize appearance: TGSongManager.addTrack hard-codes RED as the
		// track color (drawn as red staff lines); use black. Optionally set
		// a bass clef so low tunings don't dangle below a treble staff.
		final String clef = (params.has("clef") && !params.get("clef").isJsonNull())
			? params.get("clef").getAsString() : null;
		final boolean percussion = params.has("percussion")
			&& !params.get("percussion").isJsonNull()
			&& params.get("percussion").getAsBoolean();
		final TGSong finalSong = song;
		final TGTrack createdTrack = track;
		TGEditorManager editor = TGEditorManager.getInstance(this.context);
		editor.runLocked(new Runnable() {
			public void run() {
				createdTrack.getColor().setR(0);
				createdTrack.getColor().setG(0);
				createdTrack.getColor().setB(0);
				if ("bass".equals(clef)) {
					for (int i = 0; i < createdTrack.countMeasures(); i++) {
						createdTrack.getMeasure(i).setClef(
							app.tuxguitar.song.models.TGMeasure.CLEF_BASS);
					}
				}
				if (percussion) {
					// Bank 128 marks the channel as percussion (TGChannel
					// .isPercussionChannel), which routes playback to the
					// MIDI drum channel; note values become drum keys.
					app.tuxguitar.song.models.TGChannel channel =
						TGDocumentManager.getInstance(BridgeService.this.context)
							.getSongManager().getChannel(finalSong, createdTrack.getChannelId());
					if (channel != null) {
						channel.setBank(app.tuxguitar.song.models.TGChannel.DEFAULT_PERCUSSION_BANK);
						channel.setProgram(app.tuxguitar.song.models.TGChannel.DEFAULT_PERCUSSION_PROGRAM);
					}
				}
			}
		});
		editor.updateSong();
		editor.redraw();

		final AtomicReference<Integer> trackNumberRef = new AtomicReference<>(track.getNumber());
		JsonObject result = new JsonObject();
		result.addProperty("trackNumber", trackNumberRef.get());
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	public JsonObject changeTuning(JsonObject params) throws RpcException {
		final int trackNumber = params.has("trackNumber") ? params.get("trackNumber").getAsInt() : 1;
		final com.google.gson.JsonArray strings = params.getAsJsonArray("strings");
		if (strings == null || strings.size() == 0) {
			throw new RpcException(RpcException.INVALID_RANGE, "strings array is required");
		}
		if (params.has("expectedRevision")) {
			long expected = params.get("expectedRevision").getAsLong();
			long current = this.revisionTracker.getRevision();
			if (expected != current) {
				throw new RpcException(RpcException.STALE_REVISION,
					"score changed: expected revision " + expected + ", current is " + current);
			}
		}

		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		TGSong song = documentManager.getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		TGTrack track = new MeasureReader().findTrack(song, trackNumber);
		if (track == null) {
			throw new RpcException(RpcException.INVALID_RANGE, "track " + trackNumber + " not found");
		}
		this.runChangeTuning(track, strings);
		JsonObject result = new JsonObject();
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	/** Runs action.track.change-tuning (undoable via the app's action config). */
	private void runChangeTuning(TGTrack track, com.google.gson.JsonArray strings) {
		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		java.util.List<app.tuxguitar.song.models.TGString> list =
			new java.util.ArrayList<app.tuxguitar.song.models.TGString>();
		for (int i = 0; i < strings.size(); i++) {
			JsonObject wire = strings.get(i).getAsJsonObject();
			app.tuxguitar.song.models.TGString string =
				documentManager.getSongManager().getFactory().newString();
			string.setNumber(wire.has("number") ? wire.get("number").getAsInt() : i + 1);
			string.setValue(wire.has("openPitch") ? wire.get("openPitch").getAsInt() : 0);
			list.add(string);
		}
		app.tuxguitar.editor.action.TGActionProcessor tuning =
			new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.track.change-tuning");
		tuning.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_TRACK, track);
		tuning.setAttribute("strings", list);
		tuning.setAttribute("offset", Integer.valueOf(track.getOffset()));
		tuning.processOnCurrentThread();
	}

	public JsonObject setTempo(JsonObject params) throws RpcException {
		if (!params.has("bpm")) {
			throw new RpcException(RpcException.INVALID_RANGE, "bpm is required");
		}
		int bpm = params.get("bpm").getAsInt();
		if (bpm < 1 || bpm > 320) {
			throw new RpcException(RpcException.INVALID_RANGE, "bpm must be 1..320");
		}
		int fromMeasure = params.has("fromMeasure") ? params.get("fromMeasure").getAsInt() : 0;

		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		TGSong song = documentManager.getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		app.tuxguitar.song.models.TGMeasureHeader header = null;
		java.util.Iterator<app.tuxguitar.song.models.TGMeasureHeader> it = song.getMeasureHeaders();
		while (it.hasNext()) {
			app.tuxguitar.song.models.TGMeasureHeader candidate = it.next();
			if (header == null && fromMeasure <= 0) {
				header = candidate; // whole song: any header satisfies the action
			}
			if (candidate.getNumber() == fromMeasure) {
				header = candidate;
			}
		}
		if (header == null) {
			throw new RpcException(RpcException.INVALID_RANGE, "measure " + fromMeasure + " not found");
		}

		// action.composition.change-tempo-range (undo-configured, lockable):
		// APPLY_TO_ALL=1 for the whole song, APPLY_TO_END=2 from a measure on.
		app.tuxguitar.editor.action.TGActionProcessor tempo =
			new app.tuxguitar.editor.action.TGActionProcessor(
				this.context, "action.composition.change-tempo-range");
		tempo.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_SONG, song);
		tempo.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_HEADER, header);
		tempo.setAttribute("applyTo", Integer.valueOf(fromMeasure <= 0 ? 1 : 2));
		tempo.setAttribute("tempoValue", Integer.valueOf(bpm));
		tempo.setAttribute("tempoBase", Integer.valueOf(4));
		tempo.setAttribute("tempoBaseDotted", Boolean.FALSE);
		tempo.processOnCurrentThread();

		JsonObject result = new JsonObject();
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	public JsonObject exportSong(JsonObject params) throws RpcException {
		String requested = (params.has("format") && !params.get("format").isJsonNull())
			? params.get("format").getAsString().toLowerCase() : "mid";
		TGSong song = TGDocumentManager.getInstance(this.context).getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}

		app.tuxguitar.io.base.TGFileFormatManager formatManager =
			app.tuxguitar.io.base.TGFileFormatManager.getInstance(this.context);
		app.tuxguitar.io.base.TGFileFormat found = null;
		StringBuilder available = new StringBuilder();
		for (app.tuxguitar.io.base.TGSongWriter writer : formatManager.findSongWriters(false)) {
			app.tuxguitar.io.base.TGFileFormat format = writer.getFileFormat();
			if (available.length() > 0) {
				available.append(", ");
			}
			available.append(format.getName());
			if (format.getName().equalsIgnoreCase(requested)) {
				found = format;
			}
			for (String extension : format.getSupportedFormats()) {
				if (extension.equalsIgnoreCase(requested)) {
					found = format;
				}
			}
		}
		if (found == null) {
			throw new RpcException(RpcException.UNSUPPORTED,
				"unknown export format '" + requested + "'; available: " + available);
		}

		// Same dispatch as File > Export > <format>: Save-As pre-set to the
		// format — the user picks the file name and location in the dialog.
		app.tuxguitar.editor.action.TGActionProcessor export =
			new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.file.save-as");
		export.setAttribute(app.tuxguitar.io.base.TGFileFormat.class.getName(), found);
		export.processOnCurrentThread();

		JsonObject result = new JsonObject();
		result.addProperty("dialogOpened", true);
		result.addProperty("format", found.getName());
		return result;
	}

	public JsonObject setRepeat(JsonObject params) throws RpcException {
		int from = params.has("fromMeasure") ? params.get("fromMeasure").getAsInt() : 1;
		int to = params.has("toMeasure") ? params.get("toMeasure").getAsInt() : from;
		int repetitions = params.has("repetitions") ? params.get("repetitions").getAsInt() : 2;

		TGDocumentManager documentManager = TGDocumentManager.getInstance(this.context);
		TGSong song = documentManager.getSong();
		if (song == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document is open in TuxGuitar");
		}
		app.tuxguitar.song.models.TGMeasureHeader fromHeader = null;
		app.tuxguitar.song.models.TGMeasureHeader toHeader = null;
		java.util.Iterator<app.tuxguitar.song.models.TGMeasureHeader> it = song.getMeasureHeaders();
		while (it.hasNext()) {
			app.tuxguitar.song.models.TGMeasureHeader header = it.next();
			if (header.getNumber() == from) {
				fromHeader = header;
			}
			if (header.getNumber() == to) {
				toHeader = header;
			}
		}
		if (fromHeader == null || toHeader == null || to < from || repetitions < 0) {
			throw new RpcException(RpcException.INVALID_RANGE,
				"invalid repeat range " + from + "-" + to);
		}

		// action.insert.open-repeat TOGGLES: only fire when the state differs.
		// Both actions carry undo controllers in the app's action config.
		if (fromHeader.isRepeatOpen() == (repetitions == 0)) {
			app.tuxguitar.editor.action.TGActionProcessor open =
				new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.insert.open-repeat");
			open.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_SONG, song);
			open.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_HEADER, fromHeader);
			open.processOnCurrentThread();
		}
		app.tuxguitar.editor.action.TGActionProcessor close =
			new app.tuxguitar.editor.action.TGActionProcessor(this.context, "action.insert.close-repeat");
		close.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_SONG, song);
		close.setAttribute(app.tuxguitar.document.TGDocumentContextAttributes.ATTRIBUTE_HEADER, toHeader);
		close.setAttribute("repeatCount", Integer.valueOf(repetitions));
		close.processOnCurrentThread();

		JsonObject result = new JsonObject();
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	public JsonObject play() throws RpcException {
		new app.tuxguitar.editor.action.TGActionProcessor(
			this.context, "action.transport.play").processOnCurrentThread();
		JsonObject result = new JsonObject();
		result.addProperty("playing", TuxGuitar.getInstance().getPlayer().isRunning());
		return result;
	}

	public JsonObject stopPlayback() throws RpcException {
		new app.tuxguitar.editor.action.TGActionProcessor(
			this.context, "action.transport.stop").processOnCurrentThread();
		JsonObject result = new JsonObject();
		result.addProperty("playing", TuxGuitar.getInstance().getPlayer().isRunning());
		return result;
	}

	public JsonObject saveCopy() throws RpcException {
		this.runOnUiThread(new Runnable() {
			public void run() {
				new app.tuxguitar.editor.action.TGActionProcessor(
					BridgeService.this.context, "action.file.save-as").processOnCurrentThread();
			}
		});
		JsonObject result = new JsonObject();
		result.addProperty("dialogOpened", true);
		return result;
	}

	public JsonObject spikeEdit() throws RpcException {
		final AtomicReference<SpikeEdit.Outcome> outcomeRef = new AtomicReference<>();
		this.runOnUiThread(new Runnable() {
			public void run() {
				final TGEditorManager editor = TGEditorManager.getInstance(BridgeService.this.context);
				editor.runLocked(new Runnable() {
					public void run() {
						SpikeEdit.Outcome outcome = SpikeEdit.run(BridgeService.this.context);
						if (outcome != null) {
							// Fires MEASURE_UPDATED synchronously -> revision bump.
							editor.updateMeasures(Collections.singletonList(outcome.measure));
						}
						outcomeRef.set(outcome);
					}
				});
				editor.redraw();
			}
		});
		SpikeEdit.Outcome outcome = outcomeRef.get();
		if (outcome == null) {
			throw new RpcException(RpcException.NO_DOCUMENT, "no document (or empty song) to edit");
		}
		JsonObject result = new JsonObject();
		result.addProperty("track", outcome.track);
		result.addProperty("measure", outcome.measure);
		result.addProperty("description", outcome.description);
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	public JsonObject undo() throws RpcException {
		return this.undoOrRedo(true);
	}

	public JsonObject redo() throws RpcException {
		return this.undoOrRedo(false);
	}

	private JsonObject undoOrRedo(final boolean isUndo) throws RpcException {
		final AtomicReference<Boolean> performedRef = new AtomicReference<>(Boolean.FALSE);
		this.runOnUiThread(new Runnable() {
			public void run() {
				final TGEditorManager editor = TGEditorManager.getInstance(BridgeService.this.context);
				editor.runLocked(new Runnable() {
					public void run() {
						try {
							TGUndoableManager undoableManager = TGUndoableManager.getInstance(BridgeService.this.context);
							TGActionContext actionContext = TGActionManager.getInstance(BridgeService.this.context).createActionContext();
							if (isUndo && undoableManager.canUndo()) {
								undoableManager.undo(actionContext);
								performedRef.set(Boolean.TRUE);
							} else if (!isUndo && undoableManager.canRedo()) {
								undoableManager.redo(actionContext);
								performedRef.set(Boolean.TRUE);
							}
						} catch (Throwable throwable) {
							// leave performed=false; surfaced to the client below
						}
					}
				});
				if (Boolean.TRUE.equals(performedRef.get())) {
					editor.updateSong();
					editor.redraw();
				}
			}
		});
		JsonObject result = new JsonObject();
		result.addProperty("performed", Boolean.TRUE.equals(performedRef.get()));
		result.addProperty("newRevision", this.revisionTracker.getRevision());
		return result;
	}

	/** Run on the UI thread and wait for completion (with a deadline). */
	private void runOnUiThread(final Runnable runnable) throws RpcException {
		final CountDownLatch latch = new CountDownLatch(1);
		final AtomicReference<Throwable> errorRef = new AtomicReference<>();
		try {
			TGSynchronizer.getInstance(this.context).executeLater(new Runnable() {
				public void run() {
					try {
						runnable.run();
					} catch (Throwable throwable) {
						errorRef.set(throwable);
					} finally {
						latch.countDown();
					}
				}
			});
			if (!latch.await(EDIT_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
				throw new RpcException(RpcException.LOCKED, "UI thread did not respond within " + EDIT_TIMEOUT_SECONDS + "s");
			}
		} catch (InterruptedException e) {
			Thread.currentThread().interrupt();
			throw new RpcException(RpcException.INTERNAL, "interrupted while waiting for the UI thread");
		}
		Throwable error = errorRef.get();
		if (error != null) {
			throw new RpcException(RpcException.EDIT_FAILED, String.valueOf(error.getMessage()));
		}
	}
}
