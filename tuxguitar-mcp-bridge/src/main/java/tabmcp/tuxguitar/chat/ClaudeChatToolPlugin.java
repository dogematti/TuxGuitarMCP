package tabmcp.tuxguitar.chat;

import app.tuxguitar.app.tools.custom.TGToolItemPlugin;
import app.tuxguitar.util.TGContext;

/** Tools-menu entry opening the embedded AI-musician chat. */
public class ClaudeChatToolPlugin extends TGToolItemPlugin {

	public String getModuleId() {
		return "tabmcp-chat";
	}

	protected String getItemName() {
		return "TabMCP: AI Musician Chat";
	}

	protected void doAction(TGContext context) {
		ChatDialog.showFor(context);
	}
}
