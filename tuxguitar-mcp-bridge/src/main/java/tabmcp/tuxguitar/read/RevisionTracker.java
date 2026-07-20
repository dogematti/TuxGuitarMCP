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

	private final AtomicLong revision = new AtomicLong(0);
	private volatile String documentId = UUID.randomUUID().toString();

	public long getRevision() {
		return this.revision.get();
	}

	public String getDocumentId() {
		return this.documentId;
	}

	public void processEvent(TGEvent event) {
		if (TGUpdateEvent.EVENT_TYPE.equals(event.getEventType())) {
			Object mode = event.getAttribute(TGUpdateEvent.PROPERTY_UPDATE_MODE);
			if (mode instanceof Integer) {
				switch ((Integer) mode) {
					case TGUpdateEvent.MEASURE_UPDATED:
					case TGUpdateEvent.SONG_UPDATED:
						this.revision.incrementAndGet();
						break;
					case TGUpdateEvent.SONG_LOADED:
						this.documentId = UUID.randomUUID().toString();
						this.revision.incrementAndGet();
						break;
					default:
						break;
				}
			}
		}
	}
}
