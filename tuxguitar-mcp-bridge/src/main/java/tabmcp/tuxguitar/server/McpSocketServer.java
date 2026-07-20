package tabmcp.tuxguitar.server;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.SecureRandom;

import app.tuxguitar.util.TGContext;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import tabmcp.tuxguitar.rpc.BridgeService;
import tabmcp.tuxguitar.rpc.RpcException;

/**
 * Loopback-only NDJSON JSON-RPC server. One authenticated client at a time,
 * served sequentially; a disconnect frees the slot for the next client.
 */
public class McpSocketServer {

	private static final int SOCKET_IDLE_TIMEOUT_MS = 5 * 60 * 1000;

	private final TGContext context;
	private final BridgeService service;
	private final Path discoveryPath;

	private ServerSocket serverSocket;
	private Thread acceptThread;
	private String token;
	private volatile boolean running;
	private volatile boolean clientConnected;
	private volatile String lastError;

	public McpSocketServer(TGContext context, BridgeService service) {
		this.context = context;
		this.service = service;
		this.discoveryPath = DiscoveryFile.defaultPath();
	}

	public void start() throws IOException {
		this.serverSocket = new ServerSocket(0, 1, InetAddress.getLoopbackAddress());
		this.token = generateToken();
		DiscoveryFile.write(this.discoveryPath, this.serverSocket.getLocalPort(), this.token);
		this.running = true;

		this.acceptThread = new Thread(new Runnable() {
			public void run() {
				acceptLoop();
			}
		}, "tabmcp-bridge-acceptor");
		this.acceptThread.setDaemon(true);
		this.acceptThread.start();

		this.log("listening on 127.0.0.1:" + this.serverSocket.getLocalPort());
	}

	public void stop() {
		this.running = false;
		DiscoveryFile.delete(this.discoveryPath);
		if (this.serverSocket != null) {
			try {
				this.serverSocket.close();
			} catch (IOException e) {
				// closing anyway
			}
			this.serverSocket = null;
		}
		if (this.acceptThread != null) {
			try {
				this.acceptThread.join(2000);
			} catch (InterruptedException e) {
				Thread.currentThread().interrupt();
			}
			this.acceptThread = null;
		}
		this.log("stopped");
	}

	public String getStatusText() {
		StringBuilder status = new StringBuilder();
		status.append("Bridge: ").append(this.running ? "listening" : "stopped");
		if (this.running && this.serverSocket != null) {
			status.append(" on 127.0.0.1:").append(this.serverSocket.getLocalPort());
		}
		status.append("\nClient: ").append(this.clientConnected ? "connected" : "not connected");
		status.append("\nProtocol: v").append(BridgeService.PROTOCOL_VERSION);
		status.append("\nPlugin: ").append(BridgeService.PLUGIN_VERSION);
		status.append("\nRevision: ").append(this.service.getRevisionTracker().getRevision());
		status.append("\nDiscovery: ").append(this.discoveryPath);
		status.append("\nLast error: ").append(this.lastError != null ? this.lastError : "none");
		return status.toString();
	}

	private void acceptLoop() {
		while (this.running) {
			try (Socket client = this.serverSocket.accept()) {
				client.setSoTimeout(SOCKET_IDLE_TIMEOUT_MS);
				client.setTcpNoDelay(true);
				this.clientConnected = true;
				this.serveClient(client);
			} catch (IOException e) {
				if (this.running) {
					this.lastError = e.getMessage();
				}
			} finally {
				this.clientConnected = false;
			}
		}
	}

	private void serveClient(Socket client) throws IOException {
		BufferedReader reader = new BufferedReader(
			new InputStreamReader(client.getInputStream(), StandardCharsets.UTF_8));
		BufferedWriter writer = new BufferedWriter(
			new OutputStreamWriter(client.getOutputStream(), StandardCharsets.UTF_8));

		boolean authenticated = false;
		String line;
		while (this.running && (line = reader.readLine()) != null) {
			JsonObject response;
			JsonElement id = null;
			try {
				JsonObject request = JsonParser.parseString(line).getAsJsonObject();
				id = request.get("id");
				String method = request.has("method") ? request.get("method").getAsString() : "";
				JsonObject params = request.has("params") && request.get("params").isJsonObject()
					? request.getAsJsonObject("params") : new JsonObject();

				if (!authenticated && !"hello".equals(method)) {
					throw new RpcException(RpcException.NOT_AUTHENTICATED, "call hello first");
				}
				if ("hello".equals(method)) {
					this.checkHello(params);
					authenticated = true;
					response = resultResponse(id, this.service.helloResult());
				} else {
					response = resultResponse(id, this.dispatch(method, params));
				}
			} catch (RpcException e) {
				response = errorResponse(id, e.getCode(), e.getMessage());
			} catch (Throwable throwable) {
				this.lastError = String.valueOf(throwable.getMessage());
				response = errorResponse(id, RpcException.INTERNAL, String.valueOf(throwable.getMessage()));
			}
			writer.write(response.toString());
			writer.write("\n");
			writer.flush();
		}
	}

	private JsonObject dispatch(String method, JsonObject params) throws RpcException {
		switch (method) {
			case "ping":
				return this.service.ping();
			case "read_song":
				return this.service.readSong();
			case "read_measures":
				return this.service.readMeasures(params);
			case "read_selection":
				return this.service.readSelection();
			case "apply_changeset":
				return this.service.applyChangeset(params);
			case "create_track":
				return this.service.createTrack(params);
			case "change_tuning":
				return this.service.changeTuning(params);
			case "set_time_signature":
				return this.service.setTimeSignature(params);
			case "set_key_signature":
				return this.service.setKeySignature(params);
			case "insert_measures":
				return this.service.insertMeasures(params);
			case "delete_measures":
				return this.service.deleteMeasures(params);
			case "set_marker":
				return this.service.setMarker(params);
			case "set_repeat":
				return this.service.setRepeat(params);
			case "set_tempo":
				return this.service.setTempo(params);
			case "export_song":
				return this.service.exportSong(params);
			case "render_midi":
				return this.service.renderMidi();
			case "toggle_action":
				return this.service.toggleAction(params);
			case "play":
				return this.service.play();
			case "play_from":
				return this.service.playFrom(params);
			case "stop":
				return this.service.stopPlayback();
			case "save_copy":
				return this.service.saveCopy();
			case "spike_edit":
				return this.service.spikeEdit();
			case "undo":
				return this.service.undo();
			case "redo":
				return this.service.redo();
			default:
				throw new RpcException(RpcException.UNSUPPORTED, "unknown method: " + method);
		}
	}

	private void checkHello(JsonObject params) throws RpcException {
		String clientToken = params.has("token") ? params.get("token").getAsString() : "";
		if (!MessageDigest.isEqual(
				clientToken.getBytes(StandardCharsets.UTF_8),
				this.token.getBytes(StandardCharsets.UTF_8))) {
			throw new RpcException(RpcException.NOT_AUTHENTICATED, "invalid token");
		}
		int clientVersion = params.has("protocolVersion") ? params.get("protocolVersion").getAsInt() : -1;
		if (clientVersion != BridgeService.PROTOCOL_VERSION) {
			throw new RpcException(RpcException.PROTOCOL_VERSION,
				"plugin speaks protocol v" + BridgeService.PROTOCOL_VERSION + ", client sent v" + clientVersion);
		}
	}

	private static JsonObject resultResponse(JsonElement id, JsonObject result) {
		JsonObject response = new JsonObject();
		response.addProperty("jsonrpc", "2.0");
		response.add("id", id);
		response.add("result", result);
		return response;
	}

	private static JsonObject errorResponse(JsonElement id, String code, String message) {
		JsonObject data = new JsonObject();
		data.addProperty("code", code);
		JsonObject error = new JsonObject();
		error.addProperty("code", -32000);
		error.addProperty("message", message != null ? message : "unknown error");
		error.add("data", data);
		JsonObject response = new JsonObject();
		response.addProperty("jsonrpc", "2.0");
		response.add("id", id);
		response.add("error", error);
		return response;
	}

	private static String generateToken() {
		byte[] bytes = new byte[32];
		new SecureRandom().nextBytes(bytes);
		StringBuilder hex = new StringBuilder(bytes.length * 2);
		for (byte b : bytes) {
			hex.append(String.format("%02x", b));
		}
		return hex.toString();
	}

	private void log(String message) {
		System.err.println("[tabmcp-bridge] " + message);
	}

	// The TGContext is unused today but kept: later phases need it for
	// selection access and per-context registration.
	protected TGContext getContext() {
		return this.context;
	}
}
