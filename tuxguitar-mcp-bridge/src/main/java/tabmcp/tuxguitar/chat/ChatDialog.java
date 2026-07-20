package tabmcp.tuxguitar.chat;

import app.tuxguitar.app.ui.TGApplication;
import app.tuxguitar.app.view.main.TGWindow;
import app.tuxguitar.app.view.util.TGDialogUtil;
import app.tuxguitar.ui.UIFactory;
import app.tuxguitar.ui.event.UIDisposeEvent;
import app.tuxguitar.ui.event.UIDisposeListener;
import app.tuxguitar.ui.event.UIKeyPressedListener;
import app.tuxguitar.ui.event.UIKeyEvent;
import app.tuxguitar.ui.event.UISelectionEvent;
import app.tuxguitar.ui.event.UISelectionListener;
import app.tuxguitar.ui.layout.UITableLayout;
import app.tuxguitar.ui.resource.UIKey;
import app.tuxguitar.ui.widget.UIButton;
import app.tuxguitar.ui.widget.UILabel;
import app.tuxguitar.ui.widget.UIPanel;
import app.tuxguitar.ui.widget.UITextArea;
import app.tuxguitar.ui.widget.UITextField;
import app.tuxguitar.ui.widget.UIWindow;
import app.tuxguitar.util.TGContext;
import app.tuxguitar.util.TGSynchronizer;

/**
 * The embedded AI-musician chat: a TuxGuitar window backed by headless
 * Claude Code turns whose tool calls hit this same TuxGuitar's bridge.
 */
public class ChatDialog {

	private static ChatDialog instance;

	private final TGContext context;
	private final ClaudeRunner runner;

	private UIWindow dialog;
	private UITextArea transcript;
	private UITextField input;
	private UIButton sendButton;
	private UIButton stopButton;
	private UIButton resetButton;
	private UILabel statusLabel;

	private boolean hasSession;
	private volatile boolean busy;

	private ChatDialog(TGContext context) {
		this.context = context;
		this.runner = new ClaudeRunner();
	}

	public static synchronized void showFor(TGContext context) {
		if (instance == null || instance.dialog == null || instance.dialog.isDisposed()) {
			instance = new ChatDialog(context);
			instance.show();
		}
	}

	private void show() {
		UIFactory uiFactory = TGApplication.getInstance(this.context).getFactory();
		UIWindow uiParent = TGWindow.getInstance(this.context).getWindow();

		UITableLayout dialogLayout = new UITableLayout();
		this.dialog = uiFactory.createWindow(uiParent, false, true);
		this.dialog.setLayout(dialogLayout);
		this.dialog.setText("AI Musician (Claude)");
		this.dialog.addDisposeListener(new UIDisposeListener() {
			public void onDispose(UIDisposeEvent event) {
				ChatDialog.this.runner.stop();
			}
		});

		this.transcript = uiFactory.createTextArea(this.dialog, true, false);
		this.transcript.setText(
			"Welcome to the studio. The score open in TuxGuitar is my instrument -\n"
			+ "ask for riffs, arrangements, analysis, or a full song. Every edit lands\n"
			+ "in the undo stack (Cmd+Z).\n\n");
		dialogLayout.set(this.transcript, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_FILL, true, true, 1, 1, 640f, 420f, null);

		this.statusLabel = uiFactory.createLabel(this.dialog);
		this.statusLabel.setText("ready");
		dialogLayout.set(this.statusLabel, 2, 1, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, true, false);

		UITableLayout inputLayout = new UITableLayout(0f);
		UIPanel inputRow = uiFactory.createPanel(this.dialog, false);
		inputRow.setLayout(inputLayout);
		dialogLayout.set(inputRow, 3, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		this.input = uiFactory.createTextField(inputRow);
		inputLayout.set(this.input, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false, 1, 1, 420f, null, null);
		this.input.addKeyPressedListener(new UIKeyPressedListener() {
			public void onKeyPressed(UIKeyEvent event) {
				if (event.getKeyCombination().contains(UIKey.ENTER)) {
					sendCurrentInput();
				}
			}
		});

		this.sendButton = uiFactory.createButton(inputRow);
		this.sendButton.setText("Send");
		this.sendButton.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				sendCurrentInput();
			}
		});
		inputLayout.set(this.sendButton, 1, 2, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		this.stopButton = uiFactory.createButton(inputRow);
		this.stopButton.setText("Stop");
		this.stopButton.setEnabled(false);
		this.stopButton.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				ChatDialog.this.runner.stop();
				setStatus("stopped");
			}
		});
		inputLayout.set(this.stopButton, 1, 3, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		this.resetButton = uiFactory.createButton(inputRow);
		this.resetButton.setText("New Session");
		this.resetButton.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				ChatDialog.this.hasSession = false;
				appendLine("--- new session ---\n");
				setStatus("ready (fresh session)");
			}
		});
		inputLayout.set(this.resetButton, 1, 4, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		TGDialogUtil.openDialog(this.dialog,
			TGDialogUtil.OPEN_STYLE_CENTER | TGDialogUtil.OPEN_STYLE_PACK);
	}

	private void sendCurrentInput() {
		if (this.busy) {
			return;
		}
		String prompt = this.input.getText();
		if (prompt == null || prompt.trim().isEmpty()) {
			return;
		}
		final String message = prompt.trim();
		this.input.setText("");
		appendLine("You: " + message + "\n");
		setBusy(true);
		setStatus("thinking...");
		final boolean continueSession = this.hasSession;
		Thread worker = new Thread(() -> runTurn(message, continueSession), "tabmcp-chat-turn");
		worker.setDaemon(true);
		worker.start();
	}

	private void runTurn(String message, boolean continueSession) {
		this.runner.runTurn(message, continueSession, new ClaudeRunner.Listener() {
			public void onInit(String model) {
				setStatus("thinking... (" + model + ")");
			}

			public void onText(String text) {
				appendLine("Claude: " + text + "\n");
			}

			public void onToolUse(String toolName) {
				String shortName = toolName.startsWith("mcp__tuxguitar__")
					? toolName.substring("mcp__tuxguitar__".length())
					: toolName;
				appendLine("  [tool] " + shortName + "\n");
				setStatus("working: " + shortName);
			}

			public void onResult(String result, boolean isError, double costUsd, long durationMs) {
				ChatDialog.this.hasSession = true;
				if (isError && result != null && !result.isEmpty()) {
					appendLine("[error] " + result + "\n");
				}
				appendLine(String.format("  [done in %.1fs]%n%n", durationMs / 1000.0));
				setStatus("ready");
				setBusy(false);
			}

			public void onFailure(String failure) {
				appendLine("[failed] " + failure + "\n\n");
				setStatus("failed - see transcript");
				setBusy(false);
			}
		});
	}

	private void appendLine(final String text) {
		onUiThread(() -> {
			if (this.transcript != null && !this.transcript.isDisposed()) {
				this.transcript.append(text);
			}
		});
	}

	private void setStatus(final String text) {
		onUiThread(() -> {
			if (this.statusLabel != null && !this.statusLabel.isDisposed()) {
				this.statusLabel.setText(text);
			}
		});
	}

	private void setBusy(final boolean value) {
		this.busy = value;
		onUiThread(() -> {
			if (this.sendButton != null && !this.sendButton.isDisposed()) {
				this.sendButton.setEnabled(!value);
			}
			if (this.stopButton != null && !this.stopButton.isDisposed()) {
				this.stopButton.setEnabled(value);
			}
		});
	}

	private void onUiThread(Runnable runnable) {
		TGSynchronizer.getInstance(this.context).executeLater(runnable);
	}
}
