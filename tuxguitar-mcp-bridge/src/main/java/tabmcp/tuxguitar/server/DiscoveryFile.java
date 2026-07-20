package tabmcp.tuxguitar.server;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.attribute.PosixFilePermissions;

import app.tuxguitar.util.TGVersion;
import com.google.gson.JsonObject;

/**
 * `~/.tuxguitar-mcp/bridge.json` — how the Rust side finds and authenticates
 * to this plugin. Written on connect, deleted on disconnect. 0600 on POSIX.
 */
public class DiscoveryFile {

	public static final int PROTOCOL_VERSION = 1;

	public static Path defaultPath() {
		return Paths.get(System.getProperty("user.home"), ".tuxguitar-mcp", "bridge.json");
	}

	public static void write(Path path, int port, String token) throws IOException {
		JsonObject json = new JsonObject();
		json.addProperty("protocolVersion", PROTOCOL_VERSION);
		json.addProperty("port", port);
		json.addProperty("token", token);
		json.addProperty("pid", ProcessHandle.current().pid());
		json.addProperty("tuxguitarVersion", TGVersion.CURRENT.getVersion());
		json.addProperty("startedAtUnix", System.currentTimeMillis() / 1000L);

		Files.createDirectories(path.getParent());
		Files.write(path, json.toString().getBytes(StandardCharsets.UTF_8));
		try {
			Files.setPosixFilePermissions(path, PosixFilePermissions.fromString("rw-------"));
		} catch (UnsupportedOperationException e) {
			// non-POSIX filesystem (Windows): the home directory ACL is the boundary
		}
	}

	public static void delete(Path path) {
		try {
			Files.deleteIfExists(path);
		} catch (IOException e) {
			// best effort; a stale file is detected via its dead pid/port
		}
	}
}
