package tabmcp.tuxguitar;

import app.tuxguitar.editor.TGEditorManager;
import app.tuxguitar.util.TGContext;
import app.tuxguitar.util.plugin.TGPlugin;
import app.tuxguitar.util.plugin.TGPluginException;
import tabmcp.tuxguitar.read.RevisionTracker;
import tabmcp.tuxguitar.rpc.BridgeService;
import tabmcp.tuxguitar.server.McpSocketServer;

/**
 * Main plugin: starts the localhost bridge socket that the TabMCP Rust
 * service connects to, and tracks the score revision.
 */
public class TabMcpPlugin implements TGPlugin {

	public static final String MODULE_ID = "tabmcp-bridge";

	/** TGContext attribute under which the running server is published. */
	public static final String ATTRIBUTE_SERVER = McpSocketServer.class.getName();

	private McpSocketServer server;
	private RevisionTracker revisionTracker;

	public String getModuleId() {
		return MODULE_ID;
	}

	public void connect(TGContext context) throws TGPluginException {
		try {
			if (this.server == null) {
				this.revisionTracker = new RevisionTracker();
				TGEditorManager.getInstance(context).addUpdateListener(this.revisionTracker);

				BridgeService service = new BridgeService(context, this.revisionTracker);
				this.server = new McpSocketServer(context, service);
				this.server.start();

				context.setAttribute(ATTRIBUTE_SERVER, this.server);
			}
		} catch (Throwable throwable) {
			throw new TGPluginException(throwable.getMessage(), throwable);
		}
	}

	public void disconnect(TGContext context) throws TGPluginException {
		try {
			if (this.server != null) {
				this.server.stop();
				this.server = null;
				context.setAttribute(ATTRIBUTE_SERVER, null);
			}
			if (this.revisionTracker != null) {
				TGEditorManager.getInstance(context).removeUpdateListener(this.revisionTracker);
				this.revisionTracker = null;
			}
		} catch (Throwable throwable) {
			throw new TGPluginException(throwable.getMessage(), throwable);
		}
	}
}
