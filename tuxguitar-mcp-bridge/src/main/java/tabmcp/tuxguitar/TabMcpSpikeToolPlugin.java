package tabmcp.tuxguitar;

import java.util.Collections;

import app.tuxguitar.app.tools.custom.TGToolItemPlugin;
import app.tuxguitar.app.util.TGMessageDialogUtil;
import app.tuxguitar.editor.TGEditorManager;
import app.tuxguitar.util.TGContext;
import tabmcp.tuxguitar.edit.SpikeEdit;

/**
 * Tools-menu entry that runs the Milestone-1 spike edit by hand, so the
 * undo behavior can be verified with Ctrl+Z without an MCP client.
 * Runs on the UI thread (TGToolItemPlugin registers the action with the
 * sync-thread interceptor).
 */
public class TabMcpSpikeToolPlugin extends TGToolItemPlugin {

	public String getModuleId() {
		return "tabmcp-bridge-spike";
	}

	protected String getItemName() {
		return "TabMCP: Spike Edit (undoable test)";
	}

	protected void doAction(TGContext context) {
		final TGEditorManager editor = TGEditorManager.getInstance(context);
		final SpikeEdit.Outcome[] outcome = new SpikeEdit.Outcome[1];
		editor.runLocked(new Runnable() {
			public void run() {
				outcome[0] = SpikeEdit.run(context);
				if (outcome[0] != null) {
					editor.updateMeasures(Collections.singletonList(outcome[0].measure));
				}
			}
		});
		editor.redraw();
		String message = (outcome[0] != null)
			? "Applied at track " + outcome[0].track + ", measure " + outcome[0].measure
				+ ":\n" + outcome[0].description + "\n\nNow press Ctrl+Z / Cmd+Z — it must fully revert."
			: "No document (or empty song) to edit.";
		TGMessageDialogUtil.infoMessage(context, "TabMCP Spike Edit", message);
	}
}
