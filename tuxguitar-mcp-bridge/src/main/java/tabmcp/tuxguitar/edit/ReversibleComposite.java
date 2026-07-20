package tabmcp.tuxguitar.edit;

import java.util.ArrayList;
import java.util.List;

import app.tuxguitar.action.TGActionContext;
import app.tuxguitar.editor.undo.TGCannotRedoException;
import app.tuxguitar.editor.undo.TGCannotUndoException;
import app.tuxguitar.editor.undo.TGUndoableEdit;

/**
 * Composite undoable that redoes in insertion order and undoes in REVERSE
 * order. TuxGuitar's own TGUndoableEditComposite iterates forward for both,
 * which breaks sequences that add measures and then fill them (undo must
 * restore content first, then remove the added measures from the end).
 */
public class ReversibleComposite implements TGUndoableEdit {

	private final List<TGUndoableEdit> edits = new ArrayList<TGUndoableEdit>();

	public void addEdit(TGUndoableEdit edit) {
		this.edits.add(edit);
	}

	public boolean isEmpty() {
		return this.edits.isEmpty();
	}

	public void redo(TGActionContext actionContext) throws TGCannotRedoException {
		for (TGUndoableEdit edit : this.edits) {
			edit.redo(actionContext);
		}
	}

	public void undo(TGActionContext actionContext) throws TGCannotUndoException {
		for (int i = this.edits.size() - 1; i >= 0; i--) {
			this.edits.get(i).undo(actionContext);
		}
	}

	public boolean canRedo() {
		for (TGUndoableEdit edit : this.edits) {
			if (!edit.canRedo()) {
				return false;
			}
		}
		return true;
	}

	public boolean canUndo() {
		for (TGUndoableEdit edit : this.edits) {
			if (!edit.canUndo()) {
				return false;
			}
		}
		return true;
	}
}
