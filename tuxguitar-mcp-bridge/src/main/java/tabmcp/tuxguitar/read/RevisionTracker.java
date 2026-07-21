package tabmcp.tuxguitar.read;

import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

import app.tuxguitar.editor.event.TGUpdateEvent;
import app.tuxguitar.event.TGEvent;
import app.tuxguitar.event.TGEventListener;

/**
 * Maintains the monotonic edit revision every write is validated against.
 * Any measure/song update bumps it; loading a different song additionally
 * rotates the document id, so a revision from one song can never validate
 * against another.
 */
public class RevisionTracker implements TGEventListener {

	/** Update events closer together than this collapse into ONE bump.
	 *  A single user/AI edit fires several MEASURE_UPDATED events (one per
	 *  touched measure); clients kept seeing the revision "jump" and
	 *  retrying. Coalescing keeps one edit = one revision step. */
	private static final long COALESCE_WINDOW_MS = 150;

	private final AtomicLong revision = new AtomicLong(0);
	private volatile String documentId = UUID.randomUUID().toString();
	private volatile long lastBumpAtMs = 0;

	public long getRevision() {
		return this.revision.get();
	}

	public String getDocumentId() {
		return this.documentId;
	}

	private void bumpCoalesced() {
		long now = System.currentTimeMillis();
		if (now - this.lastBumpAtMs >= COALESCE_WINDOW_MS) {
			this.revision.incrementAndGet();
		}
		this.lastBumpAtMs = now;
	}

	public void processEvent(TGEvent event) {
		if (TGUpdateEvent.EVENT_TYPE.equals(event.getEventType())) {
			Object mode = event.getAttribute(TGUpdateEvent.PROPERTY_UPDATE_MODE);
			if (mode instanceof Integer) {
				switch ((Integer) mode) {
					case TGUpdateEvent.MEASURE_UPDATED:
					case TGUpdateEvent.SONG_UPDATED:
						this.bumpCoalesced();
						break;
					case TGUpdateEvent.SONG_LOADED:
						this.documentId = UUID.randomUUID().toString();
						this.revision.incrementAndGet();
						this.lastBumpAtMs = System.currentTimeMillis();
						break;
					default:
						break;
				}
			}
		}
	}
}
