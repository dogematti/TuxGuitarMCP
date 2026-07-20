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
	public static final String PLUGIN_VERSION = "0.3.0";

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
