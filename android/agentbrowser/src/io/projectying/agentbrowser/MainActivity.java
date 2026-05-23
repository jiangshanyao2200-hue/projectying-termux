package io.projectying.agentbrowser;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.DownloadManager;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.res.ColorStateList;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.os.Handler;
import android.os.Looper;
import android.os.Message;
import android.text.Editable;
import android.text.InputType;
import android.text.TextUtils;
import android.util.Patterns;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowManager;
import android.view.inputmethod.EditorInfo;
import android.webkit.CookieManager;
import android.webkit.DownloadListener;
import android.webkit.URLUtil;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;
import org.json.JSONTokener;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.RandomAccessFile;
import java.util.ArrayList;
import java.util.Locale;

public class MainActivity extends Activity {
    private static final String PREFS_NAME = "projectying_browser";
    private static final String PREF_TAB_STATE = "tab_state";
    private static final String PREF_ACTIVE_TAB_INDEX = "active_tab_index";
    private static final String PREF_NEXT_TAB_SERIAL = "next_tab_serial";
    private static final String PREF_COMMAND_OFFSET = "command_offset";
    private static final int FILE_CHOOSER_REQUEST = 2001;
    private static final int MAX_TABS = 12;
    private static final long BRIDGE_POLL_INTERVAL_MS = 1600L;
    private static final String HOME_URL = "file:///android_asset/start.html";
    private static final String BRIDGE_DIR_NAME = "projectying";
    private static final String BRIDGE_STATE_FILE = "browser_state.json";
    private static final String BRIDGE_COMMAND_FILE = "browser_commands.jsonl";
    private static final String BRIDGE_SIGNAL_FILE = "manual_signal.json";
    private static final String BRIDGE_SERVER_CALL_FILE = "server_call_request.json";
    private static final String BRIDGE_ACTION_RESULT_FILE = "browser_action_result.json";
    private static final String BRIDGE_AI_REPLY_FILE = "latest_ai_reply.txt";
    private static final String BRIDGE_SHELL_CONFIG_FILE = "shell_config.json";
    private static final String BRIDGE_EXTERNAL_START_FILE = "start.html";

    private final ArrayList<BrowserTab> tabs = new ArrayList<BrowserTab>();
    private final Handler bridgeHandler = new Handler(Looper.getMainLooper());
    private final Runnable bridgePollRunnable = new Runnable() {
        @Override
        public void run() {
            pollBridge();
            bridgeHandler.postDelayed(this, BRIDGE_POLL_INTERVAL_MS);
        }
    };

    private SharedPreferences prefs;
    private FrameLayout webContainer;
    private ScrollView aiMirrorScroll;
    private EditText urlInput;
    private EditText callNoteInput;
    private TextView statusView;
    private Button backButton;
    private Button forwardButton;
    private Button refreshButton;
    private Button addTabButton;
    private Button closeTabButton;
    private Button tabListButton;
    private Button sunButton;
    private Button callSendButton;
    private ProgressBar progressBar;
    private TextView aiMirrorView;
    private ValueCallback<Uri[]> fileChooserCallback;
    private File bridgeRoot;
    private File bridgeStateFile;
    private File bridgeCommandFile;
    private File bridgeSignalFile;
    private File bridgeServerCallFile;
    private File bridgeActionResultFile;
    private File bridgeAiReplyFile;
    private File bridgeShellConfigFile;
    private File bridgeExternalStartFile;
    private int activeTabIndex = -1;
    private int nextTabSerial = 1;
    private boolean aiControlMode;
    private String aiControlReason = "";
    private String userAgentMode = "android";
    private long commandFileOffset;
    private long lastAiReplyModified = -1L;
    private long lastBridgeStateModified = -1L;
    private long lastShellConfigModified = -1L;
    private long lastBackPressAt;
    private long lastManualSignalAt;
    private ShellConfig shellConfig = ShellConfig.defaults();
    private LinearLayout callComposerRow;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        prefs = getSharedPreferences(PREFS_NAME, MODE_PRIVATE);
        nextTabSerial = Math.max(1, prefs.getInt(PREF_NEXT_TAB_SERIAL, 1));
        commandFileOffset = Math.max(0L, prefs.getLong(PREF_COMMAND_OFFSET, 0L));
        configureWindow();
        resolveBridgeFiles();
        buildUi();
        ensureBridgeDefaults();
        loadShellConfigIfChanged(true);
        restoreTabsOrCreateInitial(getIntent());
        startBridgeLoop();
        exportBrowserState("launch");
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        String incoming = extractIntentUrl(intent);
        if (!TextUtils.isEmpty(incoming)) {
            navigateTo(normalizeTarget(incoming));
        }
    }

    @Override
    public void onConfigurationChanged(android.content.res.Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        refreshTopBar();
        exportBrowserState("configuration_changed");
    }

    @Override
    protected void onPause() {
        super.onPause();
        CookieManager.getInstance().flush();
        persistTabs();
        prefs.edit().putLong(PREF_COMMAND_OFFSET, commandFileOffset).apply();
    }

    @Override
    protected void onDestroy() {
        bridgeHandler.removeCallbacksAndMessages(null);
        for (int index = tabs.size() - 1; index >= 0; index -= 1) {
            destroyTab(tabs.get(index));
        }
        tabs.clear();
        super.onDestroy();
    }

    @Override
    public void onBackPressed() {
        BrowserTab tab = currentTab();
        if (tab != null && tab.webView.canGoBack()) {
            tab.webView.goBack();
            return;
        }

        long now = System.currentTimeMillis();
        if (now - lastBackPressAt < 1400L) {
            super.onBackPressed();
            return;
        }

        lastBackPressAt = now;
        toast("再按一次返回键退出浏览器");
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != FILE_CHOOSER_REQUEST || fileChooserCallback == null) {
            return;
        }

        Uri[] result = null;
        if (resultCode == RESULT_OK && data != null) {
            Uri selected = data.getData();
            if (selected != null) {
                result = new Uri[] { selected };
            }
        }

        fileChooserCallback.onReceiveValue(result);
        fileChooserCallback = null;
    }

    private void configureWindow() {
        Window window = getWindow();
        window.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            window.setStatusBarColor(Color.parseColor("#050A13"));
            window.setNavigationBarColor(Color.parseColor("#050A13"));
        }
    }

    private void resolveBridgeFiles() {
        bridgeRoot = resolveBridgeRoot();
        bridgeStateFile = new File(bridgeRoot, BRIDGE_STATE_FILE);
        bridgeCommandFile = new File(bridgeRoot, BRIDGE_COMMAND_FILE);
        bridgeSignalFile = new File(bridgeRoot, BRIDGE_SIGNAL_FILE);
        bridgeServerCallFile = new File(bridgeRoot, BRIDGE_SERVER_CALL_FILE);
        bridgeActionResultFile = new File(bridgeRoot, BRIDGE_ACTION_RESULT_FILE);
        bridgeAiReplyFile = new File(bridgeRoot, BRIDGE_AI_REPLY_FILE);
        bridgeShellConfigFile = new File(bridgeRoot, BRIDGE_SHELL_CONFIG_FILE);
        bridgeExternalStartFile = new File(bridgeRoot, BRIDGE_EXTERNAL_START_FILE);
    }

    private File resolveBridgeRoot() {
        File[] mediaDirs = getExternalMediaDirs();
        if (mediaDirs != null) {
            for (File dir : mediaDirs) {
                if (dir == null) {
                    continue;
                }
                File root = new File(dir, BRIDGE_DIR_NAME);
                if (root.exists() || root.mkdirs()) {
                    return root;
                }
            }
        }

        File externalFiles = getExternalFilesDir(null);
        if (externalFiles != null) {
            File root = new File(externalFiles, BRIDGE_DIR_NAME);
            if (root.exists() || root.mkdirs()) {
                return root;
            }
        }

        File root = new File(getFilesDir(), BRIDGE_DIR_NAME);
        if (!root.exists()) {
            root.mkdirs();
        }
        return root;
    }

    @SuppressLint("SetTextI18n")
    private void buildUi() {
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.parseColor("#050A13"));
        root.setLayoutParams(new ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        ));

        urlInput = new EditText(this);
        urlInput.setHint(shellConfig.urlHint);
        urlInput.setSingleLine(true);
        urlInput.setInputType(InputType.TYPE_TEXT_VARIATION_URI);
        urlInput.setImeOptions(EditorInfo.IME_ACTION_GO);
        urlInput.setTextColor(Color.parseColor("#E2E8F0"));
        urlInput.setHintTextColor(Color.parseColor("#5F728D"));
        urlInput.setTypeface(Typeface.MONOSPACE);
        urlInput.setTextSize(TypedValue.COMPLEX_UNIT_SP, 13);
        urlInput.setBackground(createPanelBackground("#0C1627", "#20324B"));
        urlInput.setPadding(dp(12), dp(10), dp(12), dp(10));
        urlInput.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            @Override
            public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                if (actionId == EditorInfo.IME_ACTION_GO || actionId == EditorInfo.IME_ACTION_DONE) {
                    navigateFromInput();
                    return true;
                }
                return false;
            }
        });
        LinearLayout.LayoutParams urlParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        );
        urlParams.setMargins(dp(6), dp(6), dp(6), dp(4));
        root.addView(urlInput, urlParams);

        progressBar = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progressBar.setMax(100);
        progressBar.setProgress(0);
        progressBar.setVisibility(View.GONE);
        progressBar.setProgressTintList(ColorStateList.valueOf(Color.parseColor("#38BDF8")));
        LinearLayout.LayoutParams progressParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            dp(2)
        );
        progressParams.setMargins(dp(6), 0, dp(6), dp(4));
        root.addView(progressBar, progressParams);

        webContainer = new FrameLayout(this);
        webContainer.setPadding(dp(1), dp(1), dp(1), dp(1));
        webContainer.setBackground(createWebFrameBackground());
        LinearLayout.LayoutParams webParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            0,
            1f
        );
        webParams.setMargins(dp(6), 0, dp(6), dp(4));
        root.addView(webContainer, webParams);

        HorizontalScrollView controlsScroll = new HorizontalScrollView(this);
        controlsScroll.setHorizontalScrollBarEnabled(false);
        controlsScroll.setFillViewport(true);
        controlsScroll.setBackground(createPanelBackground("#09111E", "#1B2A43"));

        LinearLayout controlsRow = new LinearLayout(this);
        controlsRow.setOrientation(LinearLayout.HORIZONTAL);
        controlsRow.setGravity(Gravity.CENTER_VERTICAL);
        controlsRow.setPadding(dp(6), dp(6), dp(6), dp(6));
        controlsRow.setMinimumWidth(getResources().getDisplayMetrics().widthPixels - dp(12));

        backButton = createIconButton("⇦", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                BrowserTab tab = currentTab();
                if (tab != null && tab.webView.canGoBack()) {
                    tab.webView.goBack();
                }
            }
        });
        controlsRow.addView(backButton);

        forwardButton = createIconButton("⇨", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                BrowserTab tab = currentTab();
                if (tab != null && tab.webView.canGoForward()) {
                    tab.webView.goForward();
                }
            }
        });
        controlsRow.addView(forwardButton);

        refreshButton = createIconButton("↻", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                BrowserTab tab = currentTab();
                if (tab == null) {
                    return;
                }
                if (TextUtils.isEmpty(tab.lastUrl) || HOME_URL.equals(tab.lastUrl) || resolveHomeUrl().equals(tab.lastUrl)) {
                    tab.webView.loadUrl(resolveHomeUrl());
                    return;
                }
                tab.webView.reload();
            }
        });
        controlsRow.addView(refreshButton);

        addTabButton = createIconButton("＋", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                createBrowserTab(resolveHomeUrl(), true);
            }
        });
        controlsRow.addView(addTabButton);

        closeTabButton = createIconButton("－", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                closeCurrentTab();
            }
        });
        controlsRow.addView(closeTabButton);

        tabListButton = createIconButton("☰", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                showTabList();
            }
        });
        controlsRow.addView(tabListButton);

        sunButton = createIconButton("◎", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                toggleCallComposer();
            }
        });
        sunButton.setBackground(createAccentBackground("#F59E0B", "#FCD34D"));
        sunButton.setTextColor(Color.parseColor("#190F00"));
        sunButton.setTextSize(TypedValue.COMPLEX_UNIT_SP, 18);
        sunButton.setPadding(dp(10), dp(6), dp(10), dp(8));
        controlsRow.addView(sunButton);
        controlsScroll.addView(controlsRow, new HorizontalScrollView.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        ));

        LinearLayout.LayoutParams controlsParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        );
        controlsParams.setMargins(dp(6), 0, dp(6), dp(4));
        root.addView(controlsScroll, controlsParams);

        statusView = new TextView(this);
        statusView.setTextColor(Color.parseColor("#B8C7DF"));
        statusView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        statusView.setPadding(dp(12), dp(8), dp(12), dp(8));
        statusView.setBackground(createPanelBackground("#09111E", "#1B2A43"));
        LinearLayout.LayoutParams statusParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        );
        statusParams.setMargins(dp(6), 0, dp(6), dp(4));
        root.addView(statusView, statusParams);

        callComposerRow = new LinearLayout(this);
        callComposerRow.setOrientation(LinearLayout.HORIZONTAL);
        callComposerRow.setGravity(Gravity.CENTER_VERTICAL);
        callComposerRow.setPadding(dp(8), dp(8), dp(8), dp(8));
        callComposerRow.setBackground(createPanelBackground("#09111E", "#3B2A10"));
        callComposerRow.setVisibility(View.GONE);

        callNoteInput = new EditText(this);
        callNoteInput.setHint("附加一句要求后发送给 Server");
        callNoteInput.setSingleLine(true);
        callNoteInput.setImeOptions(EditorInfo.IME_ACTION_SEND);
        callNoteInput.setInputType(InputType.TYPE_CLASS_TEXT);
        callNoteInput.setTextColor(Color.parseColor("#F8FAFC"));
        callNoteInput.setHintTextColor(Color.parseColor("#8B9CB6"));
        callNoteInput.setTypeface(Typeface.MONOSPACE);
        callNoteInput.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        callNoteInput.setBackground(createPanelBackground("#0B1422", "#2D405E"));
        callNoteInput.setPadding(dp(10), dp(8), dp(10), dp(8));
        callNoteInput.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            @Override
            public boolean onEditorAction(TextView v, int actionId, KeyEvent event) {
                if (actionId == EditorInfo.IME_ACTION_SEND || actionId == EditorInfo.IME_ACTION_DONE) {
                    submitServerCallRequest();
                    return true;
                }
                return false;
            }
        });
        LinearLayout.LayoutParams noteParams = new LinearLayout.LayoutParams(
            0,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            1f
        );
        callComposerRow.addView(callNoteInput, noteParams);

        callSendButton = createIconButton("SEND", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                submitServerCallRequest();
            }
        });
        callSendButton.setMinWidth(dp(64));
        callSendButton.setMinimumWidth(dp(64));
        callSendButton.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        callComposerRow.addView(callSendButton);

        LinearLayout.LayoutParams callParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        );
        callParams.setMargins(dp(6), 0, dp(6), dp(4));
        root.addView(callComposerRow, callParams);

        aiMirrorScroll = new ScrollView(this);
        aiMirrorScroll.setBackground(createPanelBackground("#07101B", "#162538"));
        aiMirrorView = new TextView(this);
        aiMirrorView.setText(shellConfig.aiPlaceholder);
        aiMirrorView.setTextColor(Color.parseColor("#D7E7FF"));
        aiMirrorView.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
        aiMirrorView.setPadding(dp(12), dp(10), dp(12), dp(10));
        aiMirrorView.setLineSpacing(0f, 1.12f);
        aiMirrorScroll.addView(aiMirrorView, new ScrollView.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        ));
        LinearLayout.LayoutParams aiParams = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            dp(shellConfig.aiPanelHeightDp)
        );
        aiParams.setMargins(dp(6), 0, dp(6), dp(6));
        root.addView(aiMirrorScroll, aiParams);

        setContentView(root);
    }

    private Button createIconButton(String label, View.OnClickListener listener) {
        Button button = new Button(this);
        button.setText(label);
        button.setAllCaps(false);
        button.setTypeface(Typeface.MONOSPACE, Typeface.BOLD);
        button.setTextSize(TypedValue.COMPLEX_UNIT_SP, 16);
        button.setGravity(Gravity.CENTER);
        button.setIncludeFontPadding(false);
        button.setSingleLine(true);
        button.setTextColor(Color.parseColor("#E5EEF9"));
        button.setBackground(createPanelBackground("#111C2D", "#25364E"));
        button.setMinWidth(dp(44));
        button.setMinimumWidth(dp(44));
        button.setMinHeight(0);
        button.setMinimumHeight(0);
        button.setPadding(dp(10), dp(6), dp(10), dp(8));
        button.setOnClickListener(listener);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        );
        params.leftMargin = dp(2);
        params.rightMargin = dp(2);
        button.setLayoutParams(params);
        return button;
    }

    private GradientDrawable createWebFrameBackground() {
        GradientDrawable drawable = new GradientDrawable(
            GradientDrawable.Orientation.TOP_BOTTOM,
            new int[] { Color.parseColor("#132033"), Color.parseColor("#060B13") }
        );
        drawable.setCornerRadius(dp(14));
        drawable.setStroke(dp(1), Color.parseColor("#28415F"));
        return drawable;
    }

    private GradientDrawable createPanelBackground(String fillColor, String strokeColor) {
        GradientDrawable drawable = new GradientDrawable();
        drawable.setShape(GradientDrawable.RECTANGLE);
        drawable.setColor(Color.parseColor(fillColor));
        drawable.setCornerRadius(dp(12));
        drawable.setStroke(dp(1), Color.parseColor(strokeColor));
        return drawable;
    }

    private GradientDrawable createAccentBackground(String startColor, String endColor) {
        GradientDrawable drawable = new GradientDrawable(
            GradientDrawable.Orientation.TL_BR,
            new int[] { Color.parseColor(startColor), Color.parseColor(endColor) }
        );
        drawable.setCornerRadius(dp(12));
        drawable.setStroke(dp(1), Color.parseColor("#FDE68A"));
        return drawable;
    }

    private int dp(int value) {
        return Math.round(TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            value,
            getResources().getDisplayMetrics()
        ));
    }

    private void startBridgeLoop() {
        bridgeHandler.removeCallbacksAndMessages(null);
        bridgeHandler.post(bridgePollRunnable);
    }

    private void pollBridge() {
        loadShellConfigIfChanged(false);
        refreshAiMirror();
        processBridgeCommands();
        refreshBridgeStatus();
        if (!bridgeStateFile.exists()) {
            exportBrowserState("bridge_state_missing");
        }
    }

    private void ensureBridgeDefaults() {
        try {
            if (bridgeRoot != null && !bridgeRoot.exists()) {
                bridgeRoot.mkdirs();
            }
            if (bridgeShellConfigFile != null && !bridgeShellConfigFile.exists()) {
                writeTextFile(bridgeShellConfigFile, ShellConfig.defaults().toJson().toString(2) + "\n");
            }
            if (bridgeExternalStartFile != null && !bridgeExternalStartFile.exists()) {
                copyAssetToFile("start.html", bridgeExternalStartFile);
            }
        } catch (Exception ignored) {
        }
    }

    private void loadShellConfigIfChanged(boolean force) {
        if (bridgeShellConfigFile == null || !bridgeShellConfigFile.exists()) {
            if (force) {
                applyShellConfig(shellConfig);
            }
            return;
        }
        long modified = bridgeShellConfigFile.lastModified();
        if (!force && modified == lastShellConfigModified) {
            return;
        }
        String raw = readTextFile(bridgeShellConfigFile, 32_000);
        if (TextUtils.isEmpty(raw)) {
            return;
        }
        try {
            shellConfig = ShellConfig.fromJson(new JSONObject(raw));
            lastShellConfigModified = modified;
            applyShellConfig(shellConfig);
            exportBrowserState("shell_config_reload");
        } catch (Exception ignored) {
        }
    }

    private void applyShellConfig(ShellConfig config) {
        if (config == null) {
            return;
        }
        if (urlInput != null) {
            urlInput.setHint(config.urlHint);
        }
        if (aiMirrorView != null && (bridgeAiReplyFile == null || !bridgeAiReplyFile.exists())) {
            aiMirrorView.setText(config.aiPlaceholder);
        }
        if (aiMirrorScroll != null) {
            ViewGroup.LayoutParams params = aiMirrorScroll.getLayoutParams();
            if (params instanceof LinearLayout.LayoutParams) {
                params.height = dp(config.aiPanelHeightDp);
                aiMirrorScroll.setLayoutParams(params);
            }
        }
        if (aiMirrorView != null && !hasFreshAiReply()) {
            aiMirrorView.setText(config.aiPlaceholder);
        }
    }

    private void refreshAiMirror() {
        if (bridgeAiReplyFile == null || !bridgeAiReplyFile.exists()) {
            if (aiMirrorView != null) {
                aiMirrorView.setText(shellConfig.aiPlaceholder);
            }
            return;
        }
        long modified = bridgeAiReplyFile.lastModified();
        if (modified == lastAiReplyModified) {
            return;
        }
        String text = readTextFile(bridgeAiReplyFile, 24_000);
        if (TextUtils.isEmpty(text)) {
            return;
        }
        lastAiReplyModified = modified;
        aiMirrorView.setText(text.trim());
    }

    private void refreshBridgeStatus() {
        if (statusView == null) {
            return;
        }
        boolean bridgeReady = bridgeRoot != null && bridgeRoot.exists();
        boolean projectyingActive = hasRecentBridgeState();
        boolean aiSynced = hasFreshAiReply();
        String bridgeText = bridgeReady ? "BRIDGE OK" : "BRIDGE WAIT";
        String runtimeText = projectyingActive ? "PYING LIVE" : "PYING WAIT";
        String syncText = aiSynced ? "TEXT SYNC" : "TEXT WAIT";
        String controlText = aiControlMode ? "AI LOCK" : "HUMAN OK";
        String reason = aiControlMode && !TextUtils.isEmpty(aiControlReason) ? " · " + aiControlReason : "";
        statusView.setText(bridgeText + " · " + runtimeText + " · " + syncText + " · " + controlText + " · UA " + userAgentMode.toUpperCase(Locale.US) + reason);
    }

    private boolean hasRecentBridgeState() {
        if (bridgeStateFile == null || !bridgeStateFile.exists()) {
            return false;
        }
        long modified = bridgeStateFile.lastModified();
        if (modified != lastBridgeStateModified) {
            lastBridgeStateModified = modified;
        }
        long ageMs = Math.max(0L, System.currentTimeMillis() - modified);
        return ageMs <= 20_000L;
    }

    private boolean hasFreshAiReply() {
        if (bridgeAiReplyFile == null || !bridgeAiReplyFile.exists()) {
            return false;
        }
        long ageMs = Math.max(0L, System.currentTimeMillis() - bridgeAiReplyFile.lastModified());
        return ageMs <= 120_000L && bridgeAiReplyFile.length() > 0L;
    }

    private void processBridgeCommands() {
        if (bridgeCommandFile == null || !bridgeCommandFile.exists()) {
            return;
        }
        long fileLength = bridgeCommandFile.length();
        if (fileLength < commandFileOffset) {
            commandFileOffset = 0L;
        }

        RandomAccessFile input = null;
        try {
            input = new RandomAccessFile(bridgeCommandFile, "r");
            input.seek(commandFileOffset);
            String line;
            while ((line = input.readLine()) != null) {
                handleBridgeCommand(decodeIsoLine(line));
            }
            commandFileOffset = input.getFilePointer();
            prefs.edit().putLong(PREF_COMMAND_OFFSET, commandFileOffset).apply();
        } catch (Exception ignored) {
        } finally {
            if (input != null) {
                try {
                    input.close();
                } catch (Exception ignored) {
                }
            }
        }
    }

    private String decodeIsoLine(String line) {
        try {
            return new String(line.getBytes("ISO-8859-1"), "UTF-8");
        } catch (Exception ignored) {
            return line;
        }
    }

    private void handleBridgeCommand(String line) {
        String trimmed = line == null ? "" : line.trim();
        if (trimmed.isEmpty()) {
            return;
        }

        try {
            JSONObject command = new JSONObject(trimmed);
            if (command.has("actions")) {
                JSONArray actions = command.optJSONArray("actions");
                if (actions != null) {
                    for (int index = 0; index < actions.length(); index += 1) {
                        JSONObject action = actions.optJSONObject(index);
                        if (action != null) {
                            applyBridgeCommand(action);
                        }
                    }
                }
                return;
            }
            applyBridgeCommand(command);
        } catch (Exception ignored) {
        }
    }

    private void applyBridgeCommand(JSONObject command) {
        String action = command.optString("action", "").trim().toLowerCase(Locale.US);
        if (TextUtils.isEmpty(action)) {
            return;
        }

        if ("lock".equals(action) || "ai_lock".equals(action)) {
            setAiControlMode(true, command.optString("reason", "AI 正在操作浏览器"));
            return;
        }

        if ("unlock".equals(action) || "human_unlock".equals(action)) {
            setAiControlMode(false, "");
            return;
        }

        if ("assist".equals(action) || "request_human".equals(action)) {
            BrowserTab assistTarget = resolveTabForCommand(command);
            if (assistTarget != null) {
                switchToTab(tabs.indexOf(assistTarget));
            }
            setAiControlMode(false, command.optString("reason", "需要人工协助"));
            exportManualSignal(currentTab(), "assist");
            return;
        }

        if ("open_many".equals(action) || "batch_open".equals(action)) {
            JSONArray urls = command.optJSONArray("urls");
            if (urls != null) {
                boolean background = command.optBoolean("background", true);
                for (int index = 0; index < urls.length() && tabs.size() < MAX_TABS; index += 1) {
                    String url = normalizeTarget(urls.optString(index, resolveHomeUrl()));
                    createBrowserTab(url, !background && index == urls.length() - 1);
                }
            }
            exportBrowserState("command_" + action);
            return;
        }

        if ("set_ua".equals(action) || "ua".equals(action)) {
            applyUserAgentMode(command.optString("mode", command.optString("ua", "android")));
            exportBrowserState("command_" + action);
            return;
        }

        if ("open".equals(action)) {
            String url = normalizeTarget(command.optString("url", resolveHomeUrl()));
            boolean background = command.optBoolean("background", false);
            createBrowserTab(url, !background);
            return;
        }

        BrowserTab target = resolveTabForCommand(command);
        if (target == null) {
            target = currentTab();
        }
        if (target == null) {
            return;
        }

        if ("navigate".equals(action)) {
            String url = normalizeTarget(command.optString("url", resolveHomeUrl()));
            target.webView.loadUrl(url);
            if (target == currentTab()) {
                urlInput.setText(url);
            }
        } else if ("click".equals(action) || "fill".equals(action) || "submit".equals(action)) {
            executeDomBridgeCommand(target, command, action);
            return;
        } else if ("switch".equals(action) || "focus".equals(action)) {
            int targetIndex = tabs.indexOf(target);
            if (targetIndex >= 0) {
                switchToTab(targetIndex);
            }
        } else if ("close".equals(action)) {
            closeTab(target);
        } else if ("reload".equals(action)) {
            target.webView.reload();
        } else if ("back".equals(action)) {
            if (target.webView.canGoBack()) {
                target.webView.goBack();
            }
        } else if ("forward".equals(action)) {
            if (target.webView.canGoForward()) {
                target.webView.goForward();
            }
        } else if ("snapshot".equals(action) || "signal".equals(action)) {
            exportManualSignal(target, "remote");
        } else if ("list".equals(action) || "list_tabs".equals(action)) {
            exportBrowserState("command_" + action);
        } else if ("copy_url".equals(action)) {
            copyCurrentUrl();
        }
        exportBrowserState("command_" + action);
    }

    private void executeDomBridgeCommand(final BrowserTab target, final JSONObject command, final String action) {
        if (target == null || target.webView == null) {
            return;
        }
        final String script = buildDomActionScript(command);
        target.webView.evaluateJavascript(script, new ValueCallback<String>() {
            @Override
            public void onReceiveValue(String value) {
                try {
                    JSONObject result = decodeJavascriptObject(value);
                    exportBridgeActionResult(command, result, target, action);
                    if (target == currentTab()) {
                        refreshTopBar();
                    }
                    exportBrowserState("command_" + action);
                } catch (Exception exception) {
                    try {
                        JSONObject fallback = new JSONObject();
                        fallback.put("ok", false);
                        fallback.put("error", "dom_action_decode_failed");
                        fallback.put("raw", safeText(value));
                        exportBridgeActionResult(command, fallback, target, action);
                    } catch (Exception ignored) {
                    }
                }
            }
        });
    }

    private void setAiControlMode(boolean enabled, String reason) {
        aiControlMode = enabled;
        aiControlReason = safeText(reason);
        setHumanControlsEnabled(!enabled);
        refreshBridgeStatus();
        exportBrowserState(enabled ? "ai_lock" : "ai_unlock");
    }

    private void setHumanControlsEnabled(boolean enabled) {
        if (urlInput != null) {
            urlInput.setEnabled(enabled);
            urlInput.setAlpha(enabled ? 1f : 0.54f);
        }
        if (callNoteInput != null) {
            callNoteInput.setEnabled(enabled);
            callNoteInput.setAlpha(enabled ? 1f : 0.54f);
        }
        Button[] buttons = new Button[] { backButton, forwardButton, refreshButton, addTabButton, closeTabButton, tabListButton, sunButton };
        for (Button button : buttons) {
            if (button != null) {
                button.setEnabled(enabled);
                button.setAlpha(enabled ? 1f : 0.54f);
            }
        }
        if (webContainer != null) {
            webContainer.setAlpha(enabled ? 1f : 0.92f);
        }
    }

    private void applyUserAgentMode(String mode) {
        String normalized = mode == null ? "android" : mode.trim().toLowerCase(Locale.US);
        if (!"windows".equals(normalized)) {
            normalized = "android";
        }
        userAgentMode = normalized;
        for (BrowserTab tab : tabs) {
            configureUserAgent(tab.webView);
            tab.webView.reload();
        }
        refreshBridgeStatus();
    }

    private void configureUserAgent(WebView webView) {
        if (webView == null) {
            return;
        }
        if ("windows".equals(userAgentMode)) {
            webView.getSettings().setUserAgentString("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36");
            webView.getSettings().setUseWideViewPort(true);
            webView.getSettings().setLoadWithOverviewMode(true);
            return;
        }
        webView.getSettings().setUserAgentString("Mozilla/5.0 (Linux; Android 14; ProjectYing Browser) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Mobile Safari/537.36");
        webView.getSettings().setUseWideViewPort(true);
        webView.getSettings().setLoadWithOverviewMode(true);
    }

    private BrowserTab resolveTabForCommand(JSONObject command) {
        String tabId = command.optString("tab_id", "").trim();
        if (!TextUtils.isEmpty(tabId)) {
            for (BrowserTab tab : tabs) {
                if (tab.id.equals(tabId)) {
                    return tab;
                }
            }
        }

        if (command.has("tab_index")) {
            int tabIndex = command.optInt("tab_index", -1);
            if (tabIndex >= 0 && tabIndex < tabs.size()) {
                return tabs.get(tabIndex);
            }
        }
        return currentTab();
    }

    private void restoreTabsOrCreateInitial(Intent intent) {
        JSONArray savedTabs = parseSavedTabs(prefs.getString(PREF_TAB_STATE, ""));
        if (savedTabs != null && savedTabs.length() > 0) {
            for (int index = 0; index < savedTabs.length(); index += 1) {
                JSONObject item = savedTabs.optJSONObject(index);
                if (item == null) {
                    continue;
                }
                String tabId = item.optString("id", nextTabId());
                String title = item.optString("title", "新标签页");
                String url = item.optString("url", resolveHomeUrl());
                createBrowserTabInternal(tabId, title, url, false);
            }
            int storedIndex = prefs.getInt(PREF_ACTIVE_TAB_INDEX, 0);
            switchToTab(Math.min(Math.max(storedIndex, 0), tabs.size() - 1));
            return;
        }

        String incoming = extractIntentUrl(intent);
        if (!TextUtils.isEmpty(incoming)) {
            createBrowserTab(normalizeTarget(incoming), true);
            return;
        }

        createBrowserTab(resolveHomeUrl(), true);
    }

    private JSONArray parseSavedTabs(String raw) {
        if (TextUtils.isEmpty(raw)) {
            return null;
        }
        try {
            return new JSONArray(raw);
        } catch (Exception ignored) {
            return null;
        }
    }

    private String extractIntentUrl(Intent intent) {
        if (intent == null) {
            return null;
        }
        if (Intent.ACTION_VIEW.equals(intent.getAction()) && intent.getData() != null) {
            return intent.getData().toString();
        }
        return null;
    }

    private void navigateFromInput() {
        Editable editable = urlInput.getText();
        String text = editable == null ? "" : editable.toString().trim();
        if (TextUtils.isEmpty(text)) {
            navigateTo(resolveHomeUrl());
            return;
        }
        if (aiControlMode) {
            toast("AI 正在操作，等待释放控制");
            return;
        }
        navigateTo(normalizeTarget(text));
    }

    private String normalizeTarget(String text) {
        String trimmed = text == null ? "" : text.trim();
        if (trimmed.isEmpty()) {
            return resolveHomeUrl();
        }
        if (trimmed.startsWith("http://") || trimmed.startsWith("https://") || trimmed.startsWith("file://")) {
            return trimmed;
        }
        if (trimmed.startsWith("about:") || trimmed.startsWith("intent:")) {
            return trimmed;
        }
        if (trimmed.contains(" ") || !looksLikeUrl(trimmed)) {
            return "https://www.baidu.com/s?wd=" + Uri.encode(trimmed);
        }
        return "https://" + trimmed;
    }

    private boolean looksLikeUrl(String text) {
        return text.contains(".") && Patterns.WEB_URL.matcher("https://" + text).matches();
    }

    private void navigateTo(String target) {
        BrowserTab tab = currentTab();
        if (tab == null) {
            createBrowserTab(target, true);
            return;
        }
        if (aiControlMode) {
            toast("AI 正在操作，等待释放控制");
            return;
        }
        tab.webView.loadUrl(target);
        tab.lastUrl = target;
        urlInput.setText(target);
        exportBrowserState("navigate");
    }

    private void createBrowserTab(String initialUrl, boolean makeActive) {
        if (tabs.size() >= MAX_TABS) {
            toast("标签页上限为 " + MAX_TABS);
            return;
        }
        String tabId = nextTabId();
        createBrowserTabInternal(tabId, "新标签页", initialUrl, makeActive);
        persistTabs();
        exportBrowserState("tab_open");
    }

    @SuppressLint("SetJavaScriptEnabled")
    private BrowserTab createBrowserTabInternal(String tabId, String title, String initialUrl, boolean makeActive) {
        WebView webView = new WebView(this);
        webView.setBackgroundColor(Color.parseColor("#02060C"));

        WebSettings settings = webView.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setDatabaseEnabled(true);
        settings.setAllowFileAccess(true);
        settings.setAllowContentAccess(true);
        settings.setUseWideViewPort(true);
        settings.setLoadWithOverviewMode(true);
        settings.setBuiltInZoomControls(true);
        settings.setDisplayZoomControls(false);
        settings.setSupportZoom(true);
        settings.setJavaScriptCanOpenWindowsAutomatically(true);
        settings.setMediaPlaybackRequiresUserGesture(false);
        settings.setGeolocationEnabled(true);
        settings.setSupportMultipleWindows(true);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            settings.setSafeBrowsingEnabled(true);
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.LOLLIPOP) {
            settings.setMixedContentMode(WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE);
            CookieManager.getInstance().setAcceptThirdPartyCookies(webView, true);
        }
        CookieManager.getInstance().setAcceptCookie(true);
        configureUserAgent(webView);

        BrowserTab tab = new BrowserTab(tabId, webView);
        tab.title = title;
        tab.lastUrl = TextUtils.isEmpty(initialUrl) ? resolveHomeUrl() : initialUrl;

        webView.setWebViewClient(new BrowserClient(tab));
        webView.setWebChromeClient(new BrowserChromeClient(tab));
        webView.setDownloadListener(new BrowserDownloadListener(tab));
        webView.setVisibility(View.GONE);

        tabs.add(tab);
        webContainer.addView(webView, new FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        ));

        if (makeActive || activeTabIndex < 0) {
            switchToTab(tabs.size() - 1);
        } else {
            webView.onPause();
        }

        webView.loadUrl(tab.lastUrl);
        return tab;
    }

    private String nextTabId() {
        String id = "t" + nextTabSerial;
        nextTabSerial += 1;
        prefs.edit().putInt(PREF_NEXT_TAB_SERIAL, nextTabSerial).apply();
        return id;
    }

    private void switchToTab(int index) {
        if (index < 0 || index >= tabs.size()) {
            return;
        }
        activeTabIndex = index;
        for (int tabIndex = 0; tabIndex < tabs.size(); tabIndex += 1) {
            BrowserTab tab = tabs.get(tabIndex);
            boolean active = tabIndex == activeTabIndex;
            tab.webView.setVisibility(active ? View.VISIBLE : View.GONE);
            if (active) {
                tab.webView.onResume();
            } else {
                tab.webView.onPause();
            }
        }
        refreshTopBar();
        persistTabs();
        exportBrowserState("tab_switch");
    }

    private void refreshTopBar() {
        BrowserTab tab = currentTab();
        if (tab == null) {
            return;
        }

        String url = TextUtils.isEmpty(tab.lastUrl) ? resolveHomeUrl() : tab.lastUrl;
        urlInput.setText(url);
        urlInput.setSelection(url.length());
        backButton.setEnabled(tab.webView.canGoBack());
        forwardButton.setEnabled(tab.webView.canGoForward());
        applyEnabledState(backButton, tab.webView.canGoBack());
        applyEnabledState(forwardButton, tab.webView.canGoForward());
        applyEnabledState(closeTabButton, tabs.size() > 1);
        applyEnabledState(refreshButton, true);
        tabListButton.setText("☰" + tabs.size());
        refreshBridgeStatus();
    }

    private void applyEnabledState(Button button, boolean enabled) {
        button.setEnabled(enabled);
        button.setAlpha(enabled ? 1f : 0.42f);
    }

    private void showTabList() {
        if (tabs.isEmpty()) {
            return;
        }
        final AlertDialog dialog = new AlertDialog.Builder(this).create();
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(Color.parseColor("#07101A"));

        LinearLayout panel = new LinearLayout(this);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(dp(12), dp(12), dp(12), dp(12));
        panel.setBackground(createPanelBackground("#07101A", "#20324B"));

        TextView title = new TextView(this);
        title.setText("Tabs · " + tabs.size());
        title.setTextColor(Color.parseColor("#E6F0FF"));
        title.setTypeface(Typeface.MONOSPACE, Typeface.BOLD);
        title.setTextSize(TypedValue.COMPLEX_UNIT_SP, 14);
        title.setPadding(0, 0, 0, dp(10));
        panel.addView(title);

        for (int index = 0; index < tabs.size(); index += 1) {
            final int targetIndex = index;
            BrowserTab tab = tabs.get(index);
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.VERTICAL);
            row.setPadding(dp(10), dp(10), dp(10), dp(10));
            row.setBackground(createPanelBackground(
                index == activeTabIndex ? "#102238" : "#0A1524",
                index == activeTabIndex ? "#4D89D8" : "#263750"
            ));
            row.setClickable(true);
            row.setFocusable(true);
            row.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View v) {
                    switchToTab(targetIndex);
                    dialog.dismiss();
                }
            });

            TextView rowTitle = new TextView(this);
            rowTitle.setText((index == activeTabIndex ? "● " : "○ ") + tab.id + " · " + collapseText(safeText(tab.title), 36));
            rowTitle.setTextColor(Color.parseColor("#E8F2FF"));
            rowTitle.setTypeface(Typeface.MONOSPACE, Typeface.BOLD);
            rowTitle.setTextSize(TypedValue.COMPLEX_UNIT_SP, 12);
            row.addView(rowTitle);

            TextView rowUrl = new TextView(this);
            rowUrl.setText(collapseText(safeText(tab.lastUrl), 80));
            rowUrl.setTextColor(Color.parseColor("#9FB2CD"));
            rowUrl.setTypeface(Typeface.MONOSPACE);
            rowUrl.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
            rowUrl.setPadding(0, dp(4), 0, 0);
            row.addView(rowUrl);

            LinearLayout.LayoutParams rowParams = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            );
            rowParams.bottomMargin = dp(8);
            panel.addView(row, rowParams);
        }

        LinearLayout actions = new LinearLayout(this);
        actions.setOrientation(LinearLayout.HORIZONTAL);
        actions.setGravity(Gravity.END);

        Button createButton = createIconButton("NEW", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                createBrowserTab(resolveHomeUrl(), true);
                dialog.dismiss();
            }
        });
        createButton.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        actions.addView(createButton);

        Button dismissButton = createIconButton("CLOSE", new View.OnClickListener() {
            @Override
            public void onClick(View v) {
                dialog.dismiss();
            }
        });
        dismissButton.setTextSize(TypedValue.COMPLEX_UNIT_SP, 11);
        actions.addView(dismissButton);

        panel.addView(actions);
        scroll.addView(panel);
        dialog.setView(scroll);
        dialog.show();
    }

    private void closeCurrentTab() {
        closeTab(currentTab());
    }

    private void closeTab(BrowserTab tab) {
        if (tab == null) {
            return;
        }

        int index = tabs.indexOf(tab);
        if (index < 0) {
            return;
        }

        destroyTab(tab);
        tabs.remove(index);
        if (tabs.isEmpty()) {
            activeTabIndex = -1;
            createBrowserTab(resolveHomeUrl(), true);
            return;
        }

        int nextIndex = Math.max(0, Math.min(index, tabs.size() - 1));
        switchToTab(nextIndex);
        persistTabs();
        exportBrowserState("tab_close");
    }

    private void destroyTab(BrowserTab tab) {
        try {
            tab.webView.stopLoading();
            tab.webView.setWebChromeClient(null);
            tab.webView.setWebViewClient(null);
            webContainer.removeView(tab.webView);
            tab.webView.loadUrl("about:blank");
            tab.webView.clearHistory();
            tab.webView.destroy();
        } catch (Exception ignored) {
        }
    }

    private BrowserTab currentTab() {
        if (activeTabIndex < 0 || activeTabIndex >= tabs.size()) {
            return null;
        }
        return tabs.get(activeTabIndex);
    }

    private void updateTabState(BrowserTab tab, String title, String url) {
        if (tab == null) {
            return;
        }
        if (!TextUtils.isEmpty(title)) {
            tab.title = title;
        }
        if (!TextUtils.isEmpty(url)) {
            tab.lastUrl = url;
        }
        tab.updatedAt = System.currentTimeMillis();
        if (tab == currentTab()) {
            refreshTopBar();
        }
        persistTabs();
        exportBrowserState("nav_update");
    }

    private void persistTabs() {
        JSONArray state = new JSONArray();
        for (BrowserTab tab : tabs) {
            JSONObject item = new JSONObject();
            try {
                item.put("id", tab.id);
                item.put("title", safeText(tab.title));
                item.put("url", safeText(tab.lastUrl));
                state.put(item);
            } catch (Exception ignored) {
            }
        }
        prefs.edit()
            .putString(PREF_TAB_STATE, state.toString())
            .putInt(PREF_ACTIVE_TAB_INDEX, Math.max(activeTabIndex, 0))
            .putInt(PREF_NEXT_TAB_SERIAL, nextTabSerial)
            .apply();
    }

    private void exportBrowserState(String reason) {
        if (bridgeStateFile == null) {
            return;
        }
        try {
            JSONObject state = new JSONObject();
            state.put("updated_at", System.currentTimeMillis());
            state.put("reason", reason);
            state.put("tab_count", tabs.size());
            state.put("active_tab_index", activeTabIndex);
            BrowserTab current = currentTab();
            state.put("active_tab_id", current == null ? JSONObject.NULL : current.id);
            state.put("bridge_root", bridgeRoot.getAbsolutePath());
            state.put("manual_signal_file", bridgeSignalFile.getAbsolutePath());
            state.put("server_call_file", bridgeServerCallFile.getAbsolutePath());
            state.put("action_result_file", bridgeActionResultFile.getAbsolutePath());
            state.put("command_file", bridgeCommandFile.getAbsolutePath());
            state.put("ai_reply_file", bridgeAiReplyFile.getAbsolutePath());
            state.put("shell_config_file", bridgeShellConfigFile.getAbsolutePath());
            state.put("external_start_file", bridgeExternalStartFile.getAbsolutePath());
            state.put("last_manual_signal_at", lastManualSignalAt);
            state.put("resolved_home_url", resolveHomeUrl());
            state.put("ai_control_mode", aiControlMode);
            state.put("ai_control_reason", aiControlReason);
            state.put("user_agent_mode", userAgentMode);
            state.put("supported_commands", "open,navigate,switch,close,reload,back,forward,snapshot,list_tabs,open_many,set_ua,lock,unlock,request_human,click,fill,submit");

            JSONArray tabArray = new JSONArray();
            for (int index = 0; index < tabs.size(); index += 1) {
                tabArray.put(buildTabJson(tabs.get(index), index));
            }
            state.put("tabs", tabArray);
            writeTextFile(bridgeStateFile, state.toString(2));
        } catch (Exception ignored) {
        }
    }

    private JSONObject buildTabJson(BrowserTab tab, int index) {
        JSONObject item = new JSONObject();
        try {
            item.put("index", index);
            item.put("id", tab.id);
            item.put("title", safeText(tab.title));
            item.put("url", safeText(tab.lastUrl));
            item.put("can_go_back", tab.webView.canGoBack());
            item.put("can_go_forward", tab.webView.canGoForward());
            item.put("loading", tab.loading);
            item.put("progress", tab.progress);
            item.put("updated_at", tab.updatedAt);
            item.put("active", index == activeTabIndex);
        } catch (Exception ignored) {
        }
        return item;
    }

    private void exportManualSignal(final BrowserTab tab, final String source) {
        if (tab == null) {
            return;
        }

        final String script = buildPageSnapshotScript();
        tab.webView.evaluateJavascript(script, new ValueCallback<String>() {
            @Override
            public void onReceiveValue(String value) {
                try {
                    JSONObject page = decodeJavascriptObject(value);
                    JSONObject payload = new JSONObject();
                    payload.put("created_at", System.currentTimeMillis());
                    payload.put("source", source);
                    payload.put("active_tab_index", activeTabIndex);
                    payload.put("active_tab_id", tab.id);
                    payload.put("tab_count", tabs.size());
                    payload.put("tab", buildTabJson(tab, tabs.indexOf(tab)));
                    payload.put("tabs", buildAllTabsJson());
                    payload.put("page", page);
                    writeTextFile(bridgeSignalFile, payload.toString(2));
                    lastManualSignalAt = System.currentTimeMillis();
                    exportBrowserState("manual_signal");
                    toast("当前页面已写入 AI 协作桥");
                } catch (Exception exception) {
                    toast("页面信息导出失败");
                }
            }
        });
    }

    private void toggleCallComposer() {
        if (callComposerRow == null) {
            return;
        }
        if (callComposerRow.getVisibility() == View.VISIBLE) {
            callComposerRow.setVisibility(View.GONE);
            if (callNoteInput != null) {
                callNoteInput.setText("");
            }
            return;
        }
        callComposerRow.setVisibility(View.VISIBLE);
        if (callNoteInput != null) {
            callNoteInput.requestFocus();
            callNoteInput.setSelection(callNoteInput.getText().length());
        }
    }

    private void submitServerCallRequest() {
        final BrowserTab tab = currentTab();
        if (tab == null) {
            toast("当前没有可发送的标签页");
            return;
        }
        final String note = callNoteInput == null ? "" : safeText(callNoteInput.getText().toString().trim());
        final String script = buildPageSnapshotScript();
        tab.webView.evaluateJavascript(script, new ValueCallback<String>() {
            @Override
            public void onReceiveValue(String value) {
                try {
                    JSONObject page = decodeJavascriptObject(value);
                    JSONObject payload = new JSONObject();
                    payload.put("created_at", System.currentTimeMillis());
                    payload.put("source", "human_call");
                    payload.put("target_persona", "server");
                    payload.put("user_note", note);
                    payload.put("active_tab_index", activeTabIndex);
                    payload.put("active_tab_id", tab.id);
                    payload.put("tab_count", tabs.size());
                    payload.put("tab", buildTabJson(tab, tabs.indexOf(tab)));
                    payload.put("tabs", buildAllTabsJson());
                    payload.put("page", page);
                    writeTextFile(bridgeServerCallFile, payload.toString(2));
                    lastManualSignalAt = System.currentTimeMillis();
                    callComposerRow.setVisibility(View.GONE);
                    if (callNoteInput != null) {
                        callNoteInput.setText("");
                    }
                    exportBrowserState("server_call");
                    toast("当前页面已发送给 Server");
                } catch (Exception exception) {
                    toast("发送页面给 Server 失败");
                }
            }
        });
    }

    private JSONArray buildAllTabsJson() {
        JSONArray tabArray = new JSONArray();
        for (int index = 0; index < tabs.size(); index += 1) {
            tabArray.put(buildTabJson(tabs.get(index), index));
        }
        return tabArray;
    }

    private JSONObject decodeJavascriptObject(String value) throws Exception {
        Object decoded = new JSONTokener(value).nextValue();
        if (decoded instanceof String) {
            return new JSONObject((String) decoded);
        }
        if (decoded instanceof JSONObject) {
            return (JSONObject) decoded;
        }
        return new JSONObject();
    }

    private String buildPageSnapshotScript() {
        return "(function(){"
            + "function clean(v){return (v||'').replace(/\\s+/g,' ').trim();}"
            + "var text=clean((document.body&&document.body.innerText)||'').slice(0,3000);"
            + "var links=Array.prototype.slice.call(document.querySelectorAll('a')).map(function(a){"
            + "return {text:clean(a.innerText||a.textContent||''),href:a.href||''};"
            + "}).filter(function(item){return item.text||item.href;}).slice(0,12);"
            + "var controls=Array.prototype.slice.call(document.querySelectorAll('input,textarea,select,button')).map(function(el){"
            + "return {tag:(el.tagName||'').toLowerCase(),type:(el.type||'').toLowerCase(),"
            + "label:clean(el.innerText||el.getAttribute('aria-label')||el.getAttribute('placeholder')||''),"
            + "name:clean(el.name||''),placeholder:clean(el.getAttribute('placeholder')||'')};"
            + "}).filter(function(item){return item.tag||item.label||item.name;}).slice(0,12);"
            + "return JSON.stringify({title:document.title||'',url:location.href||'',text_excerpt:text,links:links,controls:controls});"
            + "})();";
    }

    private String buildDomActionScript(JSONObject command) {
        String commandJson = JSONObject.quote(command.toString());
        return "(function(){"
            + "function clean(v){return (v||'').replace(/\\s+/g,' ').trim();}"
            + "function lower(v){return clean(v).toLowerCase();}"
            + "function visible(el){if(!el){return false;}var style=window.getComputedStyle(el);if(!style){return true;}return style.display!=='none'&&style.visibility!=='hidden';}"
            + "function textOf(el){return clean(el.innerText||el.textContent||el.value||'');}"
            + "function attr(el,name){return clean(el.getAttribute(name)||'');}"
            + "function labelFor(el){"
            + "var direct=attr(el,'aria-label')||attr(el,'placeholder');"
            + "if(direct){return direct;}"
            + "var id=attr(el,'id');"
            + "if(id){var label=document.querySelector('label[for=\"'+id.replace(/\"/g,'\\\\\"')+'\"]');if(label){return textOf(label);}}"
            + "var parent=el.closest('label');"
            + "return parent?textOf(parent):'';"
            + "}"
            + "function collectCandidates(action){"
            + "if(action==='fill'){return Array.prototype.slice.call(document.querySelectorAll('input,textarea,select'));}"
            + "if(action==='submit'){return Array.prototype.slice.call(document.querySelectorAll('form,button,input[type=submit],input[type=button]'));}"
            + "return Array.prototype.slice.call(document.querySelectorAll('a,button,input[type=button],input[type=submit],[role=button],summary'));"
            + "}"
            + "function matches(el, idx, cmd){"
            + "if(cmd.selector){try{return el.matches(cmd.selector);}catch(err){return false;}}"
            + "if(cmd.element_index!==undefined&&cmd.element_index!==null&&idx!==cmd.element_index){return false;}"
            + "var haystack=[textOf(el),labelFor(el),attr(el,'name'),attr(el,'placeholder'),attr(el,'href'),attr(el,'value')].join(' | ').toLowerCase();"
            + "if(cmd.match_text&&haystack.indexOf(String(cmd.match_text).toLowerCase())===-1){return false;}"
            + "if(cmd.href_contains&&attr(el,'href').toLowerCase().indexOf(String(cmd.href_contains).toLowerCase())===-1){return false;}"
            + "if(cmd.field_name&&attr(el,'name').toLowerCase().indexOf(String(cmd.field_name).toLowerCase())===-1){return false;}"
            + "if(cmd.placeholder&&attr(el,'placeholder').toLowerCase().indexOf(String(cmd.placeholder).toLowerCase())===-1&&labelFor(el).toLowerCase().indexOf(String(cmd.placeholder).toLowerCase())===-1){return false;}"
            + "return true;"
            + "}"
            + "function selectOption(el, value){"
            + "var target=String(value||'').toLowerCase();"
            + "var options=Array.prototype.slice.call(el.options||[]);"
            + "for(var i=0;i<options.length;i++){"
            + "var option=options[i];"
            + "if(String(option.value||'').toLowerCase()===target||clean(option.textContent||'').toLowerCase().indexOf(target)!==-1){"
            + "el.value=option.value;"
            + "return true;"
            + "}"
            + "}"
            + "return false;"
            + "}"
            + "function describe(el){return {tag:(el.tagName||'').toLowerCase(),text:textOf(el),label:labelFor(el),name:attr(el,'name'),placeholder:attr(el,'placeholder'),href:attr(el,'href'),value:attr(el,'value')};}"
            + "var cmd=JSON.parse(" + commandJson + ");"
            + "var action=String(cmd.action||'').toLowerCase();"
            + "var candidates=collectCandidates(action).filter(visible);"
            + "var target=null;var targetIndex=-1;"
            + "for(var i=0;i<candidates.length;i++){if(matches(candidates[i], i, cmd)){target=candidates[i];targetIndex=i;break;}}"
            + "if(!target&&candidates.length>0&&!cmd.selector&&!cmd.match_text&&!cmd.href_contains&&!cmd.field_name&&!cmd.placeholder&&action!=='submit'){target=candidates[0];targetIndex=0;}"
            + "if(action==='submit'&&!target){target=document.querySelector('form');targetIndex=0;}"
            + "if(!target){return JSON.stringify({ok:false,action:action,error:'target_not_found',candidate_count:candidates.length,url:location.href,title:document.title});}"
            + "if(action==='fill'){"
            + "var value=cmd.value===undefined?'':String(cmd.value);"
            + "var tag=(target.tagName||'').toLowerCase();"
            + "if(tag==='select'){selectOption(target,value);}else{target.focus();target.value=value;}"
            + "target.dispatchEvent(new Event('input',{bubbles:true}));"
            + "target.dispatchEvent(new Event('change',{bubbles:true}));"
            + "return JSON.stringify({ok:true,action:action,target_index:targetIndex,url:location.href,title:document.title,target:describe(target),value:value});"
            + "}"
            + "if(action==='submit'){"
            + "if((target.tagName||'').toLowerCase()==='form'){if(target.requestSubmit){target.requestSubmit();}else{target.submit();}}"
            + "else if(target.form){if(target.form.requestSubmit){target.form.requestSubmit(target);}else{target.form.submit();}}"
            + "else{target.click();}"
            + "return JSON.stringify({ok:true,action:action,target_index:targetIndex,url:location.href,title:document.title,target:describe(target)});"
            + "}"
            + "target.click();"
            + "return JSON.stringify({ok:true,action:action,target_index:targetIndex,url:location.href,title:document.title,target:describe(target)});"
            + "})();";
    }

    private void exportBridgeActionResult(JSONObject command, JSONObject result, BrowserTab target, String action) {
        if (bridgeActionResultFile == null) {
            return;
        }
        try {
            JSONObject payload = new JSONObject();
            payload.put("created_at", System.currentTimeMillis());
            payload.put("command_id", command.optString("command_id", ""));
            payload.put("action", action);
            payload.put("tab", target == null ? JSONObject.NULL : buildTabJson(target, tabs.indexOf(target)));
            payload.put("result", result);
            writeTextFile(bridgeActionResultFile, payload.toString(2));
        } catch (Exception ignored) {
        }
    }

    private String readTextFile(File file, int maxChars) {
        FileInputStream input = null;
        try {
            input = new FileInputStream(file);
            byte[] buffer = new byte[(int) Math.min(Math.max(file.length(), 1L), 64 * 1024L)];
            int read = input.read(buffer);
            if (read <= 0) {
                return "";
            }
            String text = new String(buffer, 0, read, "UTF-8");
            if (text.length() > maxChars) {
                return text.substring(0, maxChars);
            }
            return text;
        } catch (Exception ignored) {
            return "";
        } finally {
            if (input != null) {
                try {
                    input.close();
                } catch (Exception ignored) {
                }
            }
        }
    }

    private void writeTextFile(File file, String text) throws Exception {
        if (file == null) {
            return;
        }
        File parent = file.getParentFile();
        if (parent != null && !parent.exists()) {
            parent.mkdirs();
        }
        FileOutputStream output = null;
        try {
            output = new FileOutputStream(file, false);
            output.write(text.getBytes("UTF-8"));
            output.flush();
        } finally {
            if (output != null) {
                try {
                    output.close();
                } catch (Exception ignored) {
                }
            }
        }
    }

    private void copyAssetToFile(String assetName, File target) throws Exception {
        InputStream input = null;
        FileOutputStream output = null;
        try {
            File parent = target.getParentFile();
            if (parent != null && !parent.exists()) {
                parent.mkdirs();
            }
            input = getAssets().open(assetName);
            output = new FileOutputStream(target, false);
            byte[] buffer = new byte[8192];
            int read;
            while ((read = input.read(buffer)) != -1) {
                output.write(buffer, 0, read);
            }
            output.flush();
        } finally {
            if (input != null) {
                try {
                    input.close();
                } catch (Exception ignored) {
                }
            }
            if (output != null) {
                try {
                    output.close();
                } catch (Exception ignored) {
                }
            }
        }
    }

    private String resolveHomeUrl() {
        if (shellConfig != null && !TextUtils.isEmpty(shellConfig.homeUrl)) {
            return normalizeConfiguredHome(shellConfig.homeUrl);
        }
        if (bridgeExternalStartFile != null && bridgeExternalStartFile.exists()) {
            return "file://" + bridgeExternalStartFile.getAbsolutePath();
        }
        return HOME_URL;
    }

    private String normalizeConfiguredHome(String homeUrl) {
        String trimmed = homeUrl == null ? "" : homeUrl.trim();
        if (trimmed.isEmpty()) {
            return HOME_URL;
        }
        if ("external_start".equalsIgnoreCase(trimmed)) {
            if (bridgeExternalStartFile != null && bridgeExternalStartFile.exists()) {
                return "file://" + bridgeExternalStartFile.getAbsolutePath();
            }
            return HOME_URL;
        }
        if (trimmed.startsWith("file://") || trimmed.startsWith("http://") || trimmed.startsWith("https://")) {
            return trimmed;
        }
        return normalizeTarget(trimmed);
    }

    private String safeText(String text) {
        return TextUtils.isEmpty(text) ? "" : text;
    }

    private String collapseText(String text, int maxChars) {
        if (TextUtils.isEmpty(text)) {
            return "";
        }
        if (text.length() <= maxChars) {
            return text;
        }
        return text.substring(0, Math.max(0, maxChars - 3)) + "...";
    }

    private void copyCurrentUrl() {
        BrowserTab tab = currentTab();
        if (tab == null || TextUtils.isEmpty(tab.lastUrl)) {
            return;
        }
        try {
            android.content.ClipboardManager clipboard =
                (android.content.ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard != null) {
                clipboard.setPrimaryClip(android.content.ClipData.newPlainText("url", tab.lastUrl));
            }
        } catch (Exception ignored) {
        }
    }

    private void toast(String message) {
        Toast.makeText(this, message, Toast.LENGTH_SHORT).show();
    }

    private class BrowserClient extends WebViewClient {
        private final BrowserTab owner;

        BrowserClient(BrowserTab owner) {
            this.owner = owner;
        }

        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            Uri uri = request.getUrl();
            String scheme = uri.getScheme();
            if ("http".equalsIgnoreCase(scheme) || "https".equalsIgnoreCase(scheme) || "file".equalsIgnoreCase(scheme)) {
                return false;
            }

            if ("intent".equalsIgnoreCase(scheme)) {
                try {
                    Intent intent = Intent.parseUri(uri.toString(), Intent.URI_INTENT_SCHEME);
                    try {
                        startActivity(intent);
                    } catch (ActivityNotFoundException missingApp) {
                        String fallbackUrl = intent.getStringExtra("browser_fallback_url");
                        if (!TextUtils.isEmpty(fallbackUrl)) {
                            view.loadUrl(fallbackUrl);
                        } else {
                            toast("目标 App 未安装，已保留当前页");
                        }
                    }
                } catch (Exception exception) {
                    toast("无法处理该跳转，已保留当前页");
                }
                return true;
            }

            try {
                startActivity(new Intent(Intent.ACTION_VIEW, uri));
                return true;
            } catch (ActivityNotFoundException exception) {
                toast("当前系统无法接管该链接");
                return true;
            }
        }

        @Override
        public void onPageStarted(WebView view, String url, Bitmap favicon) {
            owner.loading = true;
            owner.lastUrl = url;
            owner.updatedAt = System.currentTimeMillis();
            if (owner == currentTab()) {
                progressBar.setVisibility(View.VISIBLE);
                progressBar.setProgress(10);
                refreshTopBar();
            }
            exportBrowserState("page_started");
        }

        @Override
        public void onPageFinished(WebView view, String url) {
            owner.loading = false;
            owner.lastUrl = url;
            owner.updatedAt = System.currentTimeMillis();
            if (owner == currentTab()) {
                progressBar.setProgress(100);
                progressBar.setVisibility(View.GONE);
                refreshTopBar();
            }
            persistTabs();
            exportBrowserState("page_finished");
        }
    }

    private class BrowserChromeClient extends WebChromeClient {
        private final BrowserTab owner;

        BrowserChromeClient(BrowserTab owner) {
            this.owner = owner;
        }

        @Override
        public void onProgressChanged(WebView view, int newProgress) {
            owner.progress = newProgress;
            owner.updatedAt = System.currentTimeMillis();
            if (owner == currentTab()) {
                progressBar.setVisibility(newProgress >= 100 ? View.GONE : View.VISIBLE);
                progressBar.setProgress(newProgress);
            }
            exportBrowserState("progress");
        }

        @Override
        public void onReceivedTitle(WebView view, String title) {
            updateTabState(owner, title, view.getUrl());
        }

        @Override
        public boolean onShowFileChooser(WebView webView, ValueCallback<Uri[]> filePathCallback, FileChooserParams fileChooserParams) {
            fileChooserCallback = filePathCallback;
            Intent chooser = fileChooserParams.createIntent();
            try {
                startActivityForResult(chooser, FILE_CHOOSER_REQUEST);
            } catch (ActivityNotFoundException exception) {
                fileChooserCallback = null;
                toast("系统没有可用的文件选择器");
                return false;
            }
            return true;
        }

        @Override
        public boolean onCreateWindow(WebView view, boolean isDialog, boolean isUserGesture, Message resultMsg) {
            if (tabs.size() >= MAX_TABS) {
                toast("标签页上限为 " + MAX_TABS);
                return false;
            }
            BrowserTab tab = createBrowserTabInternal(nextTabId(), "新标签页", resolveHomeUrl(), true);
            WebView.WebViewTransport transport = (WebView.WebViewTransport) resultMsg.obj;
            transport.setWebView(tab.webView);
            resultMsg.sendToTarget();
            return true;
        }

        @Override
        public void onCloseWindow(WebView window) {
            for (BrowserTab tab : tabs) {
                if (tab.webView == window) {
                    closeTab(tab);
                    return;
                }
            }
        }
    }

    private class BrowserDownloadListener implements DownloadListener {
        private final BrowserTab owner;

        BrowserDownloadListener(BrowserTab owner) {
            this.owner = owner;
        }

        @Override
        public void onDownloadStart(String url, String userAgent, String contentDisposition, String mimetype, long contentLength) {
            try {
                DownloadManager.Request request = new DownloadManager.Request(Uri.parse(url));
                request.setMimeType(mimetype);
                request.setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED);
                request.setTitle(URLUtil.guessFileName(url, contentDisposition, mimetype));
                request.setDescription("ProjectYing Browser 下载中 · " + owner.id);
                request.addRequestHeader("User-Agent", userAgent);

                String cookies = CookieManager.getInstance().getCookie(url);
                if (!TextUtils.isEmpty(cookies)) {
                    request.addRequestHeader("Cookie", cookies);
                }

                request.setDestinationInExternalFilesDir(
                    MainActivity.this,
                    Environment.DIRECTORY_DOWNLOADS,
                    URLUtil.guessFileName(url, contentDisposition, mimetype)
                );

                DownloadManager manager = (DownloadManager) getSystemService(DOWNLOAD_SERVICE);
                if (manager != null) {
                    manager.enqueue(request);
                    toast("已加入下载队列");
                } else {
                    toast("系统下载服务不可用");
                }
            } catch (Exception exception) {
                toast("下载失败: " + exception.getMessage());
            }
        }
    }

    private static class BrowserTab {
        final String id;
        final WebView webView;
        String title = "新标签页";
        String lastUrl = HOME_URL;
        boolean loading;
        int progress;
        long updatedAt = System.currentTimeMillis();

        BrowserTab(String id, WebView webView) {
            this.id = id;
            this.webView = webView;
        }
    }

    private static class ShellConfig {
        final String homeUrl;
        final String urlHint;
        final String aiPlaceholder;
        final int aiPanelHeightDp;

        ShellConfig(String homeUrl, String urlHint, String aiPlaceholder, int aiPanelHeightDp) {
            this.homeUrl = homeUrl;
            this.urlHint = urlHint;
            this.aiPlaceholder = aiPlaceholder;
            this.aiPanelHeightDp = Math.max(72, Math.min(aiPanelHeightDp, 220));
        }

        static ShellConfig defaults() {
            return new ShellConfig(
                "external_start",
                "网址或搜索",
                "等待 ProjectYing 启动并同步正文…",
                104
            );
        }

        static ShellConfig fromJson(JSONObject json) {
            ShellConfig defaults = defaults();
            return new ShellConfig(
                json.optString("home_url", defaults.homeUrl),
                json.optString("url_hint", defaults.urlHint),
                json.optString("ai_placeholder", defaults.aiPlaceholder),
                json.optInt("ai_panel_height_dp", defaults.aiPanelHeightDp)
            );
        }

        JSONObject toJson() {
            JSONObject json = new JSONObject();
            try {
                json.put("home_url", homeUrl);
                json.put("url_hint", urlHint);
                json.put("ai_placeholder", aiPlaceholder);
                json.put("ai_panel_height_dp", aiPanelHeightDp);
                json.put("notes", "home_url 可填 external_start、http(s) 链接或普通搜索词；桥目录下如存在 start.html，会被当作外置首页。");
            } catch (Exception ignored) {
            }
            return json;
        }
    }
}
