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
import app.tuxguitar.ui.resource.UIColor;
import app.tuxguitar.ui.resource.UIFont;
import app.tuxguitar.ui.resource.UIKey;
import app.tuxguitar.ui.widget.UIButton;
import app.tuxguitar.ui.widget.UIDropDownSelect;
import app.tuxguitar.ui.widget.UIIndeterminateProgressBar;
import app.tuxguitar.ui.widget.UILabel;
import app.tuxguitar.ui.widget.UIPanel;
import app.tuxguitar.ui.widget.UISelectItem;
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

	/** Quick-action templates: picked from the dropdown into the input. */
	private static final String[][] TEMPLATES = {
		{ "Quick actions...", "" },
		{ "Evaluate the score", "Run tuxguitar_evaluate on the whole score and fix the top issue." },
		{ "One refine pass", "Run one AI Ear refinement pass: evaluate, fix the top issue, re-evaluate, and tell me what changed." },
		{ "Hook-check this riff", "Run tuxguitar_hook_check on the current selection (or measures 1-4 of track 1) and revise until it passes." },
		{ "Producer notes", "Run tuxguitar_producer_notes and apply the best suggestion." },
		{ "Write a groove riff", "Write an 8-bar groove metal riff on the current track with generate_riff, gate it through hook_check, then generate interlocked drums and bass, humanize, and play it." },
		{ "Vary the selection", "Take the selected measures and give me three variations: one displaced, one inverted, one regrouped 3+3+2. Write them into the following measures and play the result." },
		{ "Make it heavier", "Make the last four bars heavier: more open low-string chug, kick unison, and a halftime feel." },
		{ "Listen and report", "Run tuxguitar_render_and_listen and tuxguitar_listen_stems, and report the loudest and quietest measures plus any mix problems." },
	};

	private final TGContext context;
	private final ClaudeRunner runner;

	private UIWindow dialog;
	private UITextArea transcript;
	private UITextField input;
	private UIButton sendButton;
	private UIButton stopButton;
	private UIButton resetButton;
	private UILabel statusLabel;
	private UIIndeterminateProgressBar progress;
	private UIDropDownSelect<String> modelSelect;
	private UIDropDownSelect<String> templateSelect;

	private boolean hasSession;
	private volatile boolean busy;
	private int turnCount;
	private double sessionCostUsd;
	private long sessionMs;

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

		// Studio look: dark terminal-style transcript, monospace type.
		UIColor transcriptBg = uiFactory.createColor(24, 24, 28);
		UIColor transcriptFg = uiFactory.createColor(224, 218, 200);
		UIFont mono = uiFactory.createFont("Menlo", 12f, false, false);
		UIFont monoSmall = uiFactory.createFont("Menlo", 10f, false, false);

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
		this.transcript.setBgColor(transcriptBg);
		this.transcript.setFgColor(transcriptFg);
		this.transcript.setFont(mono);
		this.transcript.setText(
			"  T A B M C P   S T U D I O\n"
			+ "  ------------------------\n"
			+ "  The score open in TuxGuitar is my instrument. Ask for riffs,\n"
			+ "  arrangements, analysis, or a full song - every edit is undoable\n"
			+ "  (Cmd+Z). Enter sends; pick a template below to start fast.\n\n");
		dialogLayout.set(this.transcript, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_FILL, true, true, 1, 1, 680f, 440f, null);

		// Status row: progress spinner + status text.
		UITableLayout statusLayout = new UITableLayout(0f);
		UIPanel statusRow = uiFactory.createPanel(this.dialog, false);
		statusRow.setLayout(statusLayout);
		dialogLayout.set(statusRow, 2, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		this.progress = uiFactory.createIndeterminateProgressBar(statusRow);
		this.progress.setVisible(false);
		statusLayout.set(this.progress, 1, 1, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, false, false, 1, 1, 90f, null, null);

		this.statusLabel = uiFactory.createLabel(statusRow);
		this.statusLabel.setFont(monoSmall);
		this.statusLabel.setText("ready");
		statusLayout.set(this.statusLabel, 1, 2, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, true, false);

		// Composer row: template picker + model picker.
		UITableLayout pickerLayout = new UITableLayout(0f);
		UIPanel pickerRow = uiFactory.createPanel(this.dialog, false);
		pickerRow.setLayout(pickerLayout);
		dialogLayout.set(pickerRow, 3, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		this.templateSelect = uiFactory.createDropDownSelect(pickerRow);
		for (String[] template : TEMPLATES) {
			this.templateSelect.addItem(new UISelectItem<String>(template[0], template[1]));
		}
		this.templateSelect.setSelectedValue("");
		this.templateSelect.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				String template = ChatDialog.this.templateSelect.getSelectedValue();
				if (template != null && !template.isEmpty()) {
					ChatDialog.this.input.setText(template);
					ChatDialog.this.input.setFocus();
				}
			}
		});
		pickerLayout.set(this.templateSelect, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		UILabel modelLabel = uiFactory.createLabel(pickerRow);
		modelLabel.setText("model:");
		pickerLayout.set(modelLabel, 1, 2, UITableLayout.ALIGN_RIGHT,
			UITableLayout.ALIGN_CENTER, false, false);

		this.modelSelect = uiFactory.createDropDownSelect(pickerRow);
		this.modelSelect.addItem(new UISelectItem<String>("default", ""));
		this.modelSelect.addItem(new UISelectItem<String>("opus", "opus"));
		this.modelSelect.addItem(new UISelectItem<String>("sonnet", "sonnet"));
		this.modelSelect.addItem(new UISelectItem<String>("haiku", "haiku"));
		this.modelSelect.setSelectedValue("");
		pickerLayout.set(this.modelSelect, 1, 3, UITableLayout.ALIGN_RIGHT,
			UITableLayout.ALIGN_CENTER, false, false);

		// Input row.
		UITableLayout inputLayout = new UITableLayout(0f);
		UIPanel inputRow = uiFactory.createPanel(this.dialog, false);
		inputRow.setLayout(inputLayout);
		dialogLayout.set(inputRow, 4, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		this.input = uiFactory.createTextField(inputRow);
		this.input.setFont(mono);
		inputLayout.set(this.input, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false, 1, 1, 440f, null, null);
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
				ChatDialog.this.turnCount = 0;
				ChatDialog.this.sessionCostUsd = 0.0;
				ChatDialog.this.sessionMs = 0L;
				appendLine("  ---------- new session ----------\n\n");
				setStatus("ready (fresh session)");
			}
		});
		inputLayout.set(this.resetButton, 1, 4, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		TGDialogUtil.openDialog(this.dialog,
			TGDialogUtil.OPEN_STYLE_CENTER | TGDialogUtil.OPEN_STYLE_PACK);
		this.input.setFocus();
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
		appendLine("> You: " + message + "\n");
		setBusy(true);
		setStatus("thinking...");
		final boolean continueSession = this.hasSession;
		final String model = this.modelSelect.getSelectedValue();
		Thread worker = new Thread(
			() -> runTurn(message, continueSession, model), "tabmcp-chat-turn");
		worker.setDaemon(true);
		worker.start();
	}

	private void runTurn(String message, boolean continueSession, String modelOverride) {
		this.runner.runTurn(message, continueSession, modelOverride,
			new ClaudeRunner.Listener() {
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
				appendLine("    * " + shortName + "\n");
				setStatus("working: " + shortName);
			}

			public void onResult(String result, boolean isError, double costUsd, long durationMs) {
				ChatDialog.this.hasSession = true;
				ChatDialog.this.turnCount++;
				ChatDialog.this.sessionCostUsd += costUsd;
				ChatDialog.this.sessionMs += durationMs;
				if (isError && result != null && !result.isEmpty()) {
					appendLine("[error] " + result + "\n");
				}
				appendLine(String.format("    -- %.1fs --%n%n", durationMs / 1000.0));
				StringBuilder status = new StringBuilder();
				status.append(String.format("ready | turn %d: %.1fs",
					ChatDialog.this.turnCount, durationMs / 1000.0));
				if (ChatDialog.this.sessionCostUsd > 0.0) {
					status.append(String.format(" | session $%.2f",
						ChatDialog.this.sessionCostUsd));
				}
				status.append(String.format(" | session %.0fs total",
					ChatDialog.this.sessionMs / 1000.0));
				setStatus(status.toString());
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
			if (this.progress != null && !this.progress.isDisposed()) {
				this.progress.setVisible(value);
			}
			if (!value && this.input != null && !this.input.isDisposed()) {
				this.input.setFocus();
			}
		});
	}

	private void onUiThread(Runnable runnable) {
		TGSynchronizer.getInstance(this.context).executeLater(runnable);
	}
}
