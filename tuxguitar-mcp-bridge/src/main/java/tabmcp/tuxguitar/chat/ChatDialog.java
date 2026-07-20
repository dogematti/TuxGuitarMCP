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
		{ "Full metalcore song", "Check the style guide for metalcore (use my player notes), then write a 20-bar song: 7-string A standard, markers Intro/Verse/Breakdown/Outro, generate_riff seeded verse with kick-accent unison, varied second half, hook-checked, interlocked breakdown drums, bass, counterline if there are gaps, humanize, one AI Ear pass with style=metalcore, then play it." },
		{ "Vary the selection", "Take the selected measures and give me three variations: one displaced, one inverted, one regrouped 3+3+2. Write them into the following measures and play the result." },
		{ "Evolve this riff", "Run tuxguitar_evolve_riff on the selection (4 generations), apply the winner, and show me the lineage." },
		{ "Extract riff DNA", "Run tuxguitar_riff_dna on the selection and save it to the DNA bank with a fitting name; then suggest one way to evolve it." },
		{ "Re-bar to 7/8", "Set the next free measures to 7/8 with set_time_signature, then rebar the selected riff into them and play both versions back to back." },
		{ "Add a counterline", "Generate a counterline answering the selected riff's gaps on a new track, humanize it, and play the result." },
		{ "Make it heavier", "Make the last four bars heavier: more open low-string chug, kick unison, and a halftime feel." },
		{ "Difficulty + realism", "Run tuxguitar_analyze_difficulty and tuxguitar_check_realism on track 1 and fix anything impossible or awkward." },
		{ "Theme map", "Run tuxguitar_track_themes and tell me whether the song remembers its own material; if not, bring an earlier motif back somewhere." },
		{ "Style match", "Run tuxguitar_style_match and tell me what this piece actually sounds like; then push it 20% closer to the nearest metal style." },
		{ "Band review", "Review the score as five band personalities (composer with track_themes, critic with hook_check, producer with producer_notes, guitarist with check_realism and analyze_difficulty, listener with render_and_listen), hold a vote, and apply the changes that get two or more votes." },
		{ "Listen and report", "Run tuxguitar_render_and_listen and tuxguitar_listen_stems, and report the loudest and quietest measures plus any mix problems." },
		{ "Save a copy", "Open the save-copy dialog so I can save this take." },
	};

	private final TGContext context;
	private final ClaudeRunner runner;

	private UIWindow dialog;
	private UITextArea transcript;
	private UITextArea input;
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
		UIFont monoInput = uiFactory.createFont("Menlo", 13f, false, false);

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

		// Input: full-width, directly under the transcript.
		this.input = uiFactory.createTextArea(this.dialog, true, false);
		this.input.setFont(monoInput);
		dialogLayout.set(this.input, 2, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_FILL, true, false, 1, 1, null, 88f, null);
		this.input.addKeyPressedListener(new UIKeyPressedListener() {
			public void onKeyPressed(UIKeyEvent event) {
				if (event.getKeyCombination().contains(UIKey.ENTER)) {
					sendCurrentInput();
				}
			}
		});

		// Bottom bar: progress + status on the left, pickers and buttons right.
		UITableLayout barLayout = new UITableLayout(0f);
		UIPanel bar = uiFactory.createPanel(this.dialog, false);
		bar.setLayout(barLayout);
		dialogLayout.set(bar, 3, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		this.progress = uiFactory.createIndeterminateProgressBar(bar);
		this.progress.setVisible(false);
		barLayout.set(this.progress, 1, 1, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, false, false, 1, 1, 80f, null, null);

		this.statusLabel = uiFactory.createLabel(bar);
		this.statusLabel.setFont(monoSmall);
		this.statusLabel.setText("ready");
		barLayout.set(this.statusLabel, 1, 2, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, true, false);

		// Stacked pickers: quick actions over the model select.
		UITableLayout rightLayout = new UITableLayout(0f);
		UIPanel rightPanel = uiFactory.createPanel(bar, false);
		rightPanel.setLayout(rightLayout);
		barLayout.set(rightPanel, 1, 3, UITableLayout.ALIGN_RIGHT,
			UITableLayout.ALIGN_CENTER, false, false);

		this.templateSelect = uiFactory.createDropDownSelect(rightPanel);
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
		rightLayout.set(this.templateSelect, 1, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false, 1, 1, 200f, null, null);

		UITableLayout modelRowLayout = new UITableLayout(0f);
		UIPanel modelRow = uiFactory.createPanel(rightPanel, false);
		modelRow.setLayout(modelRowLayout);
		rightLayout.set(modelRow, 2, 1, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false);

		UILabel modelLabel = uiFactory.createLabel(modelRow);
		modelLabel.setText("model:");
		modelRowLayout.set(modelLabel, 1, 1, UITableLayout.ALIGN_LEFT,
			UITableLayout.ALIGN_CENTER, false, false);

		this.modelSelect = uiFactory.createDropDownSelect(modelRow);
		this.modelSelect.addItem(new UISelectItem<String>("default", ""));
		this.modelSelect.addItem(new UISelectItem<String>("fable 5", "claude-fable-5"));
		this.modelSelect.addItem(new UISelectItem<String>("opus", "opus"));
		this.modelSelect.addItem(new UISelectItem<String>("sonnet", "sonnet"));
		this.modelSelect.addItem(new UISelectItem<String>("haiku", "haiku"));
		this.modelSelect.setSelectedValue("");
		modelRowLayout.set(this.modelSelect, 1, 2, UITableLayout.ALIGN_FILL,
			UITableLayout.ALIGN_CENTER, true, false, 1, 1, 130f, null, null);

		this.sendButton = uiFactory.createButton(bar);
		this.sendButton.setText("Send");
		this.sendButton.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				sendCurrentInput();
			}
		});
		barLayout.set(this.sendButton, 1, 4, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		this.stopButton = uiFactory.createButton(bar);
		this.stopButton.setText("Stop");
		this.stopButton.setEnabled(false);
		this.stopButton.addSelectionListener(new UISelectionListener() {
			public void onSelect(UISelectionEvent event) {
				ChatDialog.this.runner.stop();
				setStatus("stopped");
			}
		});
		barLayout.set(this.stopButton, 1, 5, UITableLayout.ALIGN_CENTER,
			UITableLayout.ALIGN_CENTER, false, false);

		this.resetButton = uiFactory.createButton(bar);
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
		barLayout.set(this.resetButton, 1, 6, UITableLayout.ALIGN_CENTER,
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
			if (prompt != null && !prompt.isEmpty()) {
				this.input.setText("");
			}
			return;
		}
		final String message = prompt.trim();
		this.input.setText("");
		onUiThread(() -> {
			if (this.input != null && !this.input.isDisposed()) {
				this.input.setText("");
			}
		});
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
