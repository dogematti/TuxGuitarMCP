package tabmcp.tuxguitar;

import app.tuxguitar.app.tools.custom.TGToolItemPlugin;
import app.tuxguitar.app.util.TGMessageDialogUtil;
import app.tuxguitar.util.TGContext;
import tabmcp.tuxguitar.server.McpSocketServer;

/** Tools-menu entry showing the bridge status. */
public class TabMcpStatusToolPlugin extends TGToolItemPlugin {

	public String getModuleId() {
		return "tabmcp-bridge-status";
	}

	protected String getItemName() {
		return "TabMCP: Bridge Status";
	}

	protected void doAction(TGContext context) {
		Object server = context.getAttribute(TabMcpPlugin.ATTRIBUTE_SERVER);
		String status = (server instanceof McpSocketServer)
			? ((McpSocketServer) server).getStatusText()
			: "Bridge: not running (tabmcp-bridge plugin disabled or failed to start)";
		TGMessageDialogUtil.infoMessage(context, "TabMCP Bridge", status);
	}
}
