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
import app.tuxguitar.song.models.TGSong;
import app.tuxguitar.util.TGContext;
import app.tuxguitar.util.TGSynchronizer;
import app.tuxguitar.util.TGVersion;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import tabmcp.tuxguitar.edit.SpikeEdit;
import tabmcp.tuxguitar.read.RevisionTracker;
import tabmcp.tuxguitar.read.SongReader;

/**
 * Implements the bridge methods (everything after authentication).
 * Called from the socket thread; anything touching the score model runs
 * under the editor lock, and anything mutating runs on the UI thread.
 */
public class BridgeService {

	public static final int PROTOCOL_VERSION = 1;
	public static final String PLUGIN_VERSION = "0.1.0";

	private static final long EDIT_TIMEOUT_SECONDS = 10;

	private final TGContext context;
	private final RevisionTracker revisionTracker;
	private final SongReader songReader;

	public BridgeService(TGContext context, RevisionTracker revisionTracker) {
		this.context = context;
		this.revisionTracker = revisionTracker;
		this.songReader = new SongReader();
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
		capabilities.add("edit");
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
