package tabmcp.tuxguitar.chat;

import java.io.BufferedReader;
import java.io.File;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Properties;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;

/**
 * Runs one Claude Code turn in headless print mode and streams events back.
 * Each turn is its own process; conversation continuity comes from
 * `--continue` against the dedicated working directory.
 */
public class ClaudeRunner {

	public interface Listener {
		void onInit(String model);
		void onText(String text);
		void onToolUse(String toolName);
		void onResult(String result, boolean isError, double costUsd, long durationMs);
		void onFailure(String message);
	}

	private static final String SYSTEM_CONTEXT =
		"You are the AI musician embedded in TuxGuitar through the TabMCP tools. "
		+ "A score is open in this very TuxGuitar instance; act on it directly with "
		+ "the tuxguitar_* tools. Keep chat replies short - do the work with tools "
		+ "and summarize what changed. Every edit is undoable with Cmd+Z.";

	private final Path workDir;
	private final Properties config;
	private volatile Process process;

	public ClaudeRunner() {
		this.workDir = Paths.get(System.getProperty("user.home"), ".tuxguitar-mcp", "chat");
		this.config = loadConfig();
	}

	private static Properties loadConfig() {
		Properties properties = new Properties();
		Path path = Paths.get(System.getProperty("user.home"), ".tuxguitar-mcp", "chat.properties");
		if (Files.isReadable(path)) {
			try (java.io.InputStream in = Files.newInputStream(path)) {
				properties.load(in);
			} catch (Exception ignored) {
			}
		}
		return properties;
	}

	/** Resolve the claude binary: config override, common paths, then a login shell. */
	private String resolveClaude() {
		String configured = this.config.getProperty("claude.path");
		if (configured != null && new File(configured).canExecute()) {
			return configured;
		}
		String home = System.getProperty("user.home");
		String[] candidates = {
			home + "/.claude/local/claude",
			"/opt/homebrew/bin/claude",
			"/usr/local/bin/claude",
			home + "/.local/bin/claude",
			home + "/.npm-global/bin/claude",
		};
		for (String candidate : candidates) {
			if (new File(candidate).canExecute()) {
				return candidate;
			}
		}
		try {
			Process which = new ProcessBuilder("/bin/zsh", "-lc", "command -v claude")
				.redirectErrorStream(true).start();
			try (BufferedReader reader = new BufferedReader(
					new InputStreamReader(which.getInputStream(), StandardCharsets.UTF_8))) {
				String line = reader.readLine();
				which.waitFor();
				if (line != null && new File(line.trim()).canExecute()) {
					return line.trim();
				}
			}
		} catch (Exception ignored) {
		}
		return null;
	}

	private String tabmcpPath() {
		String configured = this.config.getProperty("tabmcp.path");
		if (configured != null && new File(configured).canExecute()) {
			return configured;
		}
		return System.getProperty("user.home") + "/.cargo/bin/tabmcp";
	}

	public boolean isRunning() {
		Process current = this.process;
		return current != null && current.isAlive();
	}

	public void stop() {
		Process current = this.process;
		if (current != null && current.isAlive()) {
			current.destroy();
		}
	}

	/** Run one turn on the CALLER's thread (call from a background thread). */
	public void runTurn(String prompt, boolean continueSession, Listener listener) {
		String claude = resolveClaude();
		if (claude == null) {
			listener.onFailure(
				"claude binary not found. Set claude.path in ~/.tuxguitar-mcp/chat.properties");
			return;
		}
		try {
			Files.createDirectories(this.workDir);
		} catch (Exception e) {
			listener.onFailure("cannot create " + this.workDir + ": " + e.getMessage());
			return;
		}
		String mcpConfig = "{\"mcpServers\":{\"tuxguitar\":{\"command\":\""
			+ tabmcpPath() + "\",\"args\":[\"serve\"]}}}";
		List<String> command = new ArrayList<>();
		command.add(claude);
		command.add("-p");
		command.add(prompt);
		command.add("--output-format");
		command.add("stream-json");
		command.add("--verbose");
		command.add("--mcp-config");
		command.add(mcpConfig);
		command.add("--strict-mcp-config");
		command.add("--allowedTools");
		command.add(this.config.getProperty("allowed.tools", "mcp__tuxguitar"));
		command.add("--append-system-prompt");
		command.add(SYSTEM_CONTEXT);
		String model = this.config.getProperty("claude.model");
		if (model != null && !model.isEmpty()) {
			command.add("--model");
			command.add(model);
		}
		if (continueSession) {
			command.add("--continue");
		}
		try {
			ProcessBuilder builder = new ProcessBuilder(command);
			builder.directory(this.workDir.toFile());
			builder.redirectErrorStream(false);
			Process started = builder.start();
			this.process = started;
			StringBuilder stderrTail = new StringBuilder();
			Thread stderrReader = new Thread(() -> {
				try (BufferedReader reader = new BufferedReader(new InputStreamReader(
						started.getErrorStream(), StandardCharsets.UTF_8))) {
					String line;
					while ((line = reader.readLine()) != null) {
						if (stderrTail.length() < 4000) {
							stderrTail.append(line).append('\n');
						}
					}
				} catch (Exception ignored) {
				}
			}, "tabmcp-chat-stderr");
			stderrReader.setDaemon(true);
			stderrReader.start();

			boolean sawResult = false;
			try (BufferedReader reader = new BufferedReader(new InputStreamReader(
					started.getInputStream(), StandardCharsets.UTF_8))) {
				String line;
				while ((line = reader.readLine()) != null) {
					sawResult |= dispatch(line, listener);
				}
			}
			int exit = started.waitFor();
			if (exit != 0 && !sawResult) {
				String tail = stderrTail.toString().trim();
				listener.onFailure("claude exited with code " + exit
					+ (tail.isEmpty() ? "" : (": " + tail)));
			}
		} catch (Exception e) {
			listener.onFailure(e.getMessage() == null ? e.toString() : e.getMessage());
		} finally {
			this.process = null;
		}
	}

	/** Parse one stream-json line; returns true when it was the final result. */
	private boolean dispatch(String line, Listener listener) {
		JsonObject event;
		try {
			JsonElement parsed = JsonParser.parseString(line);
			if (!parsed.isJsonObject()) {
				return false;
			}
			event = parsed.getAsJsonObject();
		} catch (Exception malformed) {
			return false;
		}
		String type = event.has("type") ? event.get("type").getAsString() : "";
		switch (type) {
			case "system": {
				String subtype = event.has("subtype") ? event.get("subtype").getAsString() : "";
				if ("init".equals(subtype) && event.has("model")) {
					listener.onInit(event.get("model").getAsString());
				}
				return false;
			}
			case "assistant": {
				JsonObject message = event.getAsJsonObject("message");
				if (message == null || !message.has("content")) {
					return false;
				}
				JsonArray content = message.getAsJsonArray("content");
				for (JsonElement blockElement : content) {
					if (!blockElement.isJsonObject()) {
						continue;
					}
					JsonObject block = blockElement.getAsJsonObject();
					String blockType = block.has("type") ? block.get("type").getAsString() : "";
					if ("text".equals(blockType) && block.has("text")) {
						String text = block.get("text").getAsString().trim();
						if (!text.isEmpty()) {
							listener.onText(text);
						}
					} else if ("tool_use".equals(blockType) && block.has("name")) {
						listener.onToolUse(block.get("name").getAsString());
					}
				}
				return false;
			}
			case "result": {
				String result = event.has("result") && !event.get("result").isJsonNull()
					? event.get("result").getAsString() : "";
				boolean isError = event.has("is_error") && event.get("is_error").getAsBoolean();
				double cost = event.has("total_cost_usd") && !event.get("total_cost_usd").isJsonNull()
					? event.get("total_cost_usd").getAsDouble() : 0.0;
				long duration = event.has("duration_ms") && !event.get("duration_ms").isJsonNull()
					? event.get("duration_ms").getAsLong() : 0L;
				listener.onResult(result, isError, cost, duration);
				return true;
			}
			default:
				return false;
		}
	}
}
