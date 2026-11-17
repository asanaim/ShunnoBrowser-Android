package com.shunnobrowser.app

import android.annotation.SuppressLint
import android.app.AlertDialog
import android.app.PictureInPictureParams
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.text.InputType
import android.util.Rational
import android.util.TypedValue
import android.view.ActionMode
import android.view.Gravity
import android.view.KeyEvent
import android.view.Menu
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.webkit.CookieManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.activity.ComponentActivity
import java.io.ByteArrayInputStream
import java.io.File
import java.net.URLEncoder

class MainActivity : ComponentActivity() {

    private lateinit var webContainer: FrameLayout
    private lateinit var urlInput: EditText
    private lateinit var progressBar: ProgressBar
    private lateinit var tabCountBtn: TextView
    private lateinit var mediaPill: LinearLayout
    private lateinit var mediaPillLabel: TextView
    private lateinit var findBar: LinearLayout
    private lateinit var findInput: EditText
    private lateinit var findCountText: TextView

    private val tabs = mutableListOf<BrowserTab>()
    private var activeTabIndex = 0
    private val detectedMedia = mutableListOf<String>()
    private var blockedAdsCount = 0

    private var globalAdBlock = true
    private var backgroundYouTubePlay = true
    private var sponsorBlockAndShortsClean = true
    private var autoRejectCookies = true
    private var stripTrackingParams = true
    private var oledForceDark = false
    private var bottomBarMode = true
    private var incognitoMode = false

    private val siteJsDisabled = mutableSetOf<String>()
    private val siteImagesDisabled = mutableSetOf<String>()
    private val siteDesktopMode = mutableSetOf<String>()
    private val bookmarks = mutableListOf<Pair<String, String>>()

    private val adHosts = setOf(
        "doubleclick.net", "googlesyndication.com", "googleadservices.com",
        "adnxs.com", "adsrvr.org", "taboola.com", "outbrain.com",
        "popads.net", "popcash.net", "propellerads.com", "exoclick.com",
        "scorecardresearch.com", "criteo.com", "pubmatic.com", "rubiconproject.com"
    )

    data class BrowserTab(
        val id: Long = System.currentTimeMillis(),
        val webView: WebView,
        var title: String = "New Tab",
        var url: String = "https://duckduckgo.com",
        val isIncognito: Boolean = false
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        loadBookmarks()
        buildCupertinoUi()

        val startUrl = extractCleanUrl(intent) ?: "https://duckduckgo.com"
        createNewTab(startUrl, switchTo = true)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        extractCleanUrl(intent)?.let { incoming ->
            createNewTab(incoming, switchTo = false)
            updateTabCounter()
            Toast.makeText(this, "Queued in background tab (${tabs.size})", Toast.LENGTH_SHORT).show()
        }
    }

    private fun extractCleanUrl(intent: Intent?): String? {
        val raw = intent?.dataString ?: intent?.getStringExtra(Intent.EXTRA_TEXT) ?: return null
        val match = Regex("https?://\S+").find(raw)?.value ?: return null
        return if (stripTrackingParams) sanitizeTrackingUrl(match) else match
    }

    private fun sanitizeTrackingUrl(rawUrl: String): String {
        return try {
            val uri = Uri.parse(rawUrl)
            val trackingKeys = setOf("utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content", "fbclid", "gclid", "si", "igshid")
            if (uri.queryParameterNames.none { it in trackingKeys }) return rawUrl
            val builder = uri.buildUpon().clearQuery()
            for (param in uri.queryParameterNames) {
                if (param !in trackingKeys) {
                    uri.getQueryParameters(param).forEach { builder.appendQueryParameter(param, it) }
                }
            }
            builder.build().toString()
        } catch (_: Exception) {
            rawUrl
        }
    }

    private fun buildCupertinoUi() {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.parseColor("#F2F2F7"))
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        }

        progressBar = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply {
            max = 100
            visibility = View.GONE
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(3))
        }

        findBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(Color.WHITE)
            setPadding(dp(12), dp(6), dp(12), dp(6))
            visibility = View.GONE
        }
        findInput = EditText(this).apply {
            hint = "Find in page..."
            textSize = 14f
            setSingleLine(true)
            background = roundRect("#F2F2F7", 10f)
            setPadding(dp(10), dp(6), dp(10), dp(6))
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
            setOnEditorActionListener { _, _, _ ->
                currentWebView()?.findAllAsync(text.toString())
                true
            }
        }
        findCountText = TextView(this).apply {
            text = "0/0"
            textSize = 12f
            setTextColor(Color.GRAY)
            setPadding(dp(8), 0, dp(8), 0)
        }
        val findNextBtn = makeChip("↓", "#EBF4FF", "#0A84FF") { currentWebView()?.findNext(true) }
        val findPrevBtn = makeChip("↑", "#EBF4FF", "#0A84FF") { currentWebView()?.findNext(false) }
        val findCloseBtn = makeChip("✕", "#FFEBEA", "#FF3B30") {
            currentWebView()?.clearMatches()
            findBar.visibility = View.GONE
        }
        findBar.addView(findInput)
        findBar.addView(findCountText)
        findBar.addView(findPrevBtn)
        findBar.addView(findNextBtn)
        findBar.addView(findCloseBtn)

        webContainer = FrameLayout(this).apply {
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f)
        }

        mediaPill = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            background = roundRect("#E61C1C1E", 22f)
            setPadding(dp(14), dp(10), dp(14), dp(10))
            visibility = View.GONE
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
            ).apply { bottomMargin = dp(14) }
        }
        mediaPillLabel = TextView(this).apply {
            text = "🎯 Stream Detected"
            setTextColor(Color.WHITE)
            textSize = 13f
            setTypeface(null, Typeface.BOLD)
        }
        val grabBtn = TextView(this).apply {
            text = "  ⚡ Shunno Get  "
            setTextColor(Color.WHITE)
            textSize = 12f
            setTypeface(null, Typeface.BOLD)
            background = roundRect("#30D158", 14f)
            setPadding(dp(8), dp(5), dp(8), dp(5))
            setOnClickListener { showDetectedStreamsDialog() }
        }
        val dismissPillBtn = TextView(this).apply {
            text = " ✕ "
            setTextColor(Color.LTGRAY)
            textSize = 13f
            setPadding(dp(6), 0, 0, 0)
            setOnClickListener { mediaPill.visibility = View.GONE }
        }
        mediaPill.addView(mediaPillLabel)
        mediaPill.addView(TextView(this).apply { text = "   " })
        mediaPill.addView(grabBtn)
        mediaPill.addView(dismissPillBtn)

        val navBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(Color.WHITE)
            setPadding(dp(8), dp(7), dp(8), dp(7))
            elevation = dp(6).toFloat()
        }

        val backBtn = makeIconBtn("‹") {
            val wv = currentWebView()
            if (wv != null && wv.canGoBack()) wv.goBack()
        }
        val fwdBtn = makeIconBtn("›") {
            val wv = currentWebView()
            if (wv != null && wv.canGoForward()) wv.goForward()
        }

        urlInput = EditText(this).apply {
            hint = "Search or enter URL..."
            textSize = 13.5f
            setTextColor(Color.parseColor("#1C1C1E"))
            setHintTextColor(Color.parseColor("#8E8E93"))
            setSingleLine(true)
            imeOptions = EditorInfo.IME_ACTION_GO
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            background = roundRect("#F2F2F7", 12f)
            setPadding(dp(12), dp(8), dp(12), dp(8))
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                leftMargin = dp(4)
                rightMargin = dp(4)
            }
            setOnEditorActionListener { _, actionId, event ->
                if (actionId == EditorInfo.IME_ACTION_GO || event?.keyCode == KeyEvent.KEYCODE_ENTER) {
                    navigateTo(text.toString())
                    true
                } else false
            }
        }

        val reloadBtn = makeIconBtn("↻") { currentWebView()?.reload() }

        tabCountBtn = TextView(this).apply {
            text = "1"
            textSize = 12f
            setTypeface(null, Typeface.BOLD)
            setTextColor(Color.parseColor("#0A84FF"))
            gravity = Gravity.CENTER
            background = roundRect("#EBF4FF", 9f)
            layoutParams = LinearLayout.LayoutParams(dp(30), dp(30)).apply {
                leftMargin = dp(4)
                rightMargin = dp(4)
            }
            setOnClickListener { showTabsSwitcherDialog() }
        }

        val menuBtn = makeIconBtn("☰") { showNbProPowerMenu() }

        navBar.addView(backBtn)
        navBar.addView(fwdBtn)
        navBar.addView(urlInput)
        navBar.addView(reloadBtn)
        navBar.addView(tabCountBtn)
        navBar.addView(menuBtn)

        root.addView(progressBar)
        root.addView(findBar)
        if (bottomBarMode) {
            root.addView(webContainer)
            root.addView(navBar)
        } else {
            root.addView(navBar)
            root.addView(webContainer)
        }

        setContentView(root)
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun createNewTab(targetUrl: String, switchTo: Boolean) {
        val wv = WebView(this).apply {
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT
            )
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = !incognitoMode
            settings.databaseEnabled = !incognitoMode
            settings.builtInZoomControls = true
            settings.displayZoomControls = false
            settings.mediaPlaybackRequiresUserGesture = false
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_COMPATIBILITY_MODE

            setFindListener { activeMatchOrdinal, numberOfMatches, _ ->
                findCountText.text = if (numberOfMatches > 0) "${activeMatchOrdinal + 1}/$numberOfMatches" else "0/0"
            }

            setDownloadListener { url, _, contentDisposition, _, _ ->
                val fileName = android.webkit.URLUtil.guessFileName(url, contentDisposition, null)
                val cookies = CookieManager.getInstance().getCookie(url)
                promptShunnoGetDownload(url, fileName, this.url, cookies)
            }

            webChromeClient = object : WebChromeClient() {
                override fun onProgressChanged(view: WebView?, newProgress: Int) {
                    progressBar.progress = newProgress
                    progressBar.visibility = if (newProgress in 1..99) View.VISIBLE else View.GONE
                }

                override fun onReceivedTitle(view: WebView?, title: String?) {
                    tabs.find { it.webView == view }?.title = title ?: "Tab"
                }
            }

            webViewClient = object : WebViewClient() {
                override fun shouldInterceptRequest(
                    view: WebView?,
                    request: WebResourceRequest?
                ): WebResourceResponse? {
                    val reqUrl = request?.url?.toString().orEmpty()
                    val host = request?.url?.host.orEmpty().lowercase()

                    if (globalAdBlock && adHosts.any { host.endsWith(it) }) {
                        blockedAdsCount++
                        return WebResourceResponse("text/plain", "utf-8", ByteArrayInputStream(ByteArray(0)))
                    }

                    val cleanPath = reqUrl.substringBefore('?').lowercase()
                    if (cleanPath.endsWith(".mp4") || cleanPath.endsWith(".m3u8") ||
                        cleanPath.endsWith(".flac") || cleanPath.endsWith(".m4a") ||
                        cleanPath.endsWith(".mp3") || cleanPath.endsWith(".zip") ||
                        cleanPath.endsWith(".apk")
                    ) {
                        runOnUiThread {
                            if (!detectedMedia.contains(reqUrl)) {
                                detectedMedia.add(0, reqUrl)
                                mediaPillLabel.text = "🎯 ${detectedMedia.size} Stream(s)"
                                mediaPill.visibility = View.VISIBLE
                            }
                        }
                    }
                    return super.shouldInterceptRequest(view, request)
                }

                override fun onPageStarted(view: WebView?, url: String?, favicon: Bitmap?) {
                    val currentUrl = url.orEmpty()
                    val host = Uri.parse(currentUrl).host.orEmpty().lowercase()
                    view?.settings?.javaScriptEnabled = host !in siteJsDisabled
                    view?.settings?.loadsImagesAutomatically = host !in siteImagesDisabled
                    if (host in siteDesktopMode) {
                        view?.settings?.userAgentString = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/128.0.0.0 Safari/537.36"
                    }

                    tabs.find { it.webView == view }?.url = currentUrl
                    if (view == currentWebView() && !urlInput.hasFocus()) {
                        urlInput.setText(currentUrl)
                    }

                    if (backgroundYouTubePlay && view != null) {
                        injectBackgroundMediaScript(view)
                    }
                }

                override fun onPageFinished(view: WebView?, url: String?) {
                    if (view == null) return
                    val currentUrl = url.orEmpty()
                    val host = Uri.parse(currentUrl).host.orEmpty().lowercase()

                    if (backgroundYouTubePlay) {
                        injectBackgroundMediaScript(view)
                        if (host.contains("youtube.com") || host.contains("youtu.be")) {
                            BackgroundMediaService.updateOrStart(this@MainActivity, "Playing YouTube in background")
                        }
                    }
                    if (sponsorBlockAndShortsClean && host.contains("youtube.com")) {
                        injectYouTubeClutterAndSponsorScript(view)
                    }
                    if (autoRejectCookies) {
                        injectCookieBannerRejector(view)
                    }
                    if (oledForceDark) {
                        injectOledForceDark(view)
                    }
                    injectCustomUserScripts(view, host)
                }
            }
        }

        val tab = BrowserTab(webView = wv, url = targetUrl, isIncognito = incognitoMode)
        tabs.add(tab)
        if (switchTo) {
            switchToTab(tabs.size - 1)
        }
        wv.loadUrl(targetUrl)
        updateTabCounter()
    }

    private fun switchToTab(index: Int) {
        if (index !in tabs.indices) return
        activeTabIndex = index
        webContainer.removeAllViews()
        val activeTab = tabs[index]
        webContainer.addView(activeTab.webView)
        webContainer.addView(mediaPill)
        urlInput.setText(activeTab.url)
        updateTabCounter()
    }

    private fun currentWebView(): WebView? = tabs.getOrNull(activeTabIndex)?.webView

    private fun updateTabCounter() {
        tabCountBtn.text = if (incognitoMode) "🕵${tabs.size}" else "${tabs.size}"
    }

    private fun navigateTo(raw: String) {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return
        val finalUrl = when {
            trimmed.startsWith("http://") || trimmed.startsWith("https://") -> trimmed
            trimmed.contains('.') && !trimmed.contains(' ') -> "https://$trimmed"
            else -> "https://duckduckgo.com/?q=${URLEncoder.encode(trimmed, "UTF-8")}"
        }
        val clean = if (stripTrackingParams) sanitizeTrackingUrl(finalUrl) else finalUrl
        detectedMedia.clear()
        mediaPill.visibility = View.GONE
        currentWebView()?.loadUrl(clean)
    }

    private fun injectBackgroundMediaScript(wv: WebView) {
        val js = "(function(){try{Object.defineProperty(document,'hidden',{get:()=>false,configurable:true});Object.defineProperty(document,'visibilityState',{get:()=>'visible',configurable:true});window.addEventListener('visibilitychange',e=>e.stopImmediatePropagation(),true);document.addEventListener('visibilitychange',e=>e.stopImmediatePropagation(),true);}catch(e){}})();"
        wv.evaluateJavascript(js, null)
    }

    private fun injectYouTubeClutterAndSponsorScript(wv: WebView) {
        val js = "(function(){const s=document.createElement('style');s.textContent='ytm-reel-shelf-renderer,ytm-rich-section-renderer,.ad-showing,.ytp-ad-module{display:none!important;}';document.head&&document.head.appendChild(s);setInterval(()=>{const b=document.querySelector('.ytp-ad-skip-button,.ytp-ad-skip-button-modern');if(b)b.click();const v=document.querySelector('video');if(v&&document.querySelector('.ad-showing')){v.playbackRate=16.0;v.muted=true;}},800);})();"
        wv.evaluateJavascript(js, null)
    }

    private fun injectCookieBannerRejector(wv: WebView) {
        val js = "(function(){['[id*="reject"]','[class*="reject"]','[aria-label*="Reject"]','#onetrust-reject-all-handler','.fc-cta-do-not-consent'].forEach(sel=>document.querySelectorAll(sel).forEach(b=>b.click()));})();"
        wv.evaluateJavascript(js, null)
    }

    private fun injectOledForceDark(wv: WebView) {
        val js = "(function(){const s=document.createElement('style');s.textContent='html,body{background:#000000!important;color:#E5E5EA!important;}img,video{filter:none!important;}';document.head&&document.head.appendChild(s);})();"
        wv.evaluateJavascript(js, null)
    }

    private fun injectCustomUserScripts(wv: WebView, host: String) {
        try {
            val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "ShunnoBrowser/scripts")
            if (!dir.exists()) return
            dir.listFiles { f -> f.extension.equals("js", true) }?.forEach { scriptFile ->
                val code = scriptFile.readText()
                if (!code.contains("@match") || code.contains(host)) {
                    wv.evaluateJavascript(code, null)
                }
            }
        } catch (_: Exception) { }
    }

    override fun onActionModeStarted(mode: ActionMode?) {
        super.onActionModeStarted(mode)
        mode?.menu?.add(Menu.NONE, 9001, 0, "📖 Shunno Dictionary")?.setOnMenuItemClickListener {
            currentWebView()?.evaluateJavascript("window.getSelection().toString()") { raw ->
                val selected = raw?.trim('"')?.trim().orEmpty()
                if (selected.isNotEmpty()) {
                    val dictIntent = Intent(Intent.ACTION_PROCESS_TEXT).apply {
                        type = "text/plain"
                        putExtra(Intent.EXTRA_PROCESS_TEXT, selected)
                        putExtra(Intent.EXTRA_PROCESS_TEXT_READONLY, true)
                    }
                    try {
                        startActivity(dictIntent)
                    } catch (_: Exception) {
                        Toast.makeText(this, "Install Shunno Dictionary for instant pop-up lookup", Toast.LENGTH_SHORT).show()
                    }
                }
            }
            mode.finish()
            true
        }
    }

    override fun onPause() {
        super.onPause()
        if (!backgroundYouTubePlay) {
            currentWebView()?.onPause()
        }
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && backgroundYouTubePlay) {
            try {
                enterPictureInPictureMode(
                    PictureInPictureParams.Builder()
                        .setAspectRatio(Rational(16, 9))
                        .build()
                )
            } catch (_: Exception) { }
        }
    }

    private fun showNbProPowerMenu() {
        val currentUrl = currentWebView()?.url.orEmpty()
        val host = Uri.parse(currentUrl).host.orEmpty().lowercase()

        val options = arrayOf(
            "⚡ Shunno Get Downloads (${ShunnoGetEngine.downloads.size})",
            "🎧 Background YouTube Audio: ${if (backgroundYouTubePlay) "ON" else "OFF"}",
            "🛡️ AdBlock (${blockedAdsCount} blocked): ${if (globalAdBlock) "ON" else "OFF"}",
            "🎬 SponsorBlock & Shorts Stripper: ${if (sponsorBlockAndShortsClean) "ON" else "OFF"}",
            "⏩ Video Playback Speed (1.0× / 1.5× / 2.0× / 3.0×)",
            "📖 Clean Reader Mode",
            "🌑 OLED Force-Dark Mode: ${if (oledForceDark) "ON" else "OFF"}",
            "💻 Site Desktop User-Agent ($host): ${if (host in siteDesktopMode) "ON" else "OFF"}",
            "⚙️ Site JavaScript ($host): ${if (host !in siteJsDisabled) "ON" else "OFF"}",
            "🖼️ Site Images ($host): ${if (host !in siteImagesDisabled) "ON" else "OFF"}",
            "🔍 Find in Page",
            "⭐ Add Bookmark / View Bookmarks",
            "🕵️ Toggle Incognito Mode: ${if (incognitoMode) "ON" else "OFF"}"
        )

        AlertDialog.Builder(this)
            .setTitle("Shunno Browser • NBPro Controls")
            .setItems(options) { _, which ->
                when (which) {
                    0 -> showShunnoGetManagerDialog()
                    1 -> {
                        backgroundYouTubePlay = !backgroundYouTubePlay
                        if (!backgroundYouTubePlay) BackgroundMediaService.stop(this)
                        Toast.makeText(this, "Background YouTube Audio: $backgroundYouTubePlay", Toast.LENGTH_SHORT).show()
                    }
                    2 -> globalAdBlock = !globalAdBlock
                    3 -> {
                        sponsorBlockAndShortsClean = !sponsorBlockAndShortsClean
                        currentWebView()?.reload()
                    }
                    4 -> showVideoSpeedPicker()
                    5 -> activateCleanReaderMode()
                    6 -> {
                        oledForceDark = !oledForceDark
                        currentWebView()?.reload()
                    }
                    7 -> {
                        if (host in siteDesktopMode) siteDesktopMode.remove(host) else siteDesktopMode.add(host)
                        currentWebView()?.reload()
                    }
                    8 -> {
                        if (host in siteJsDisabled) siteJsDisabled.remove(host) else siteJsDisabled.add(host)
                        currentWebView()?.reload()
                    }
                    9 -> {
                        if (host in siteImagesDisabled) siteImagesDisabled.remove(host) else siteImagesDisabled.add(host)
                        currentWebView()?.reload()
                    }
                    10 -> {
                        findBar.visibility = View.VISIBLE
                        findInput.requestFocus()
                    }
                    11 -> showBookmarksDialog()
                    12 -> {
                        incognitoMode = !incognitoMode
                        createNewTab("https://duckduckgo.com", switchTo = true)
                    }
                }
            }
            .show()
    }

    private fun showVideoSpeedPicker() {
        val speeds = arrayOf("0.75×", "1.0× (Normal)", "1.25×", "1.5×", "2.0×", "3.0×")
        val values = floatArrayOf(0.75f, 1.0f, 1.25f, 1.5f, 2.0f, 3.0f)
        AlertDialog.Builder(this)
            .setTitle("HTML5 Video Speed Controller")
            .setItems(speeds) { _, idx ->
                val rate = values[idx]
                currentWebView()?.evaluateJavascript(
                    "document.querySelectorAll('video,audio').forEach(m => m.playbackRate = $rate);",
                    null
                )
            }
            .show()
    }

    private fun activateCleanReaderMode() {
        val js = "(function(){const t=(document.querySelector('article')||document.body).innerText;document.body.innerHTML='<div style="max-width:680px;margin:24px auto;padding:16px;font-family:-apple-system,sans-serif;font-size:19px;line-height:1.65;color:#1C1C1E;background:#F2F2F7;">'+t.split('\n').filter(p=>p.trim().length>0).map(p=>'<p style="margin-bottom:16px;">'+p+'</p>').join('')+'</div>';})();"
        currentWebView()?.evaluateJavascript(js, null)
    }

    private fun showDetectedStreamsDialog() {
        if (detectedMedia.isEmpty()) return
        val labels = detectedMedia.map { it.substringAfterLast('/').substringBefore('?') }.toTypedArray()
        AlertDialog.Builder(this)
            .setTitle("Sniffed Media & Files (${detectedMedia.size})")
            .setItems(labels) { _, idx ->
                val targetUrl = detectedMedia[idx]
                val fileName = labels[idx].ifBlank { "stream_${System.currentTimeMillis() % 10000}.mp4" }
                val cookies = CookieManager.getInstance().getCookie(currentWebView()?.url)
                promptShunnoGetDownload(targetUrl, fileName, currentWebView()?.url, cookies)
            }
            .show()
    }

    private fun promptShunnoGetDownload(url: String, fileName: String, referer: String?, cookie: String?) {
        val threadOptions = arrayOf("8 Parallel Threads (Recommended)", "16 Parallel Threads (Max Speed)", "4 Threads", "1 Stream")
        val threadCounts = intArrayOf(8, 16, 4, 1)
        AlertDialog.Builder(this)
            .setTitle("⚡ Shunno Get • $fileName")
            .setItems(threadOptions) { _, idx ->
                val rec = DownloadRecord(
                    id = System.currentTimeMillis(),
                    url = url,
                    fileName = fileName,
                    threads = threadCounts[idx],
                    category = ShunnoGetEngine.detectCategory(fileName),
                    referer = referer,
                    cookie = cookie
                )
                ShunnoGetEngine.downloads.add(0, rec)
                ShunnoGetEngine.start(this, rec) { }
                Toast.makeText(this, "Downloading $fileName (${rec.threads} threads)", Toast.LENGTH_SHORT).show()
            }
            .show()
    }

    private fun showShunnoGetManagerDialog() {
        val scroll = ScrollView(this)
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(12), dp(16), dp(12))
        }
        if (ShunnoGetEngine.downloads.isEmpty()) {
            box.addView(TextView(this).apply {
                text = "No downloads yet. Tap any file link or use the floating Stream Pill."
                textSize = 14f
            })
        } else {
            for (d in ShunnoGetEngine.downloads) {
                val mbDone = d.downloadedBytes / (1024 * 1024)
                val mbTot = if (d.totalBytes > 0) "${d.totalBytes / (1024 * 1024)} MB" else "?"
                val speedKb = d.speedBps / 1024
                val row = TextView(this).apply {
                    text = "• ${d.fileName}
   ${d.status} • $mbDone / $mbTot MB (${d.threads}T • ${speedKb} KB/s)
   [Tap to Pause/Resume]"
                    textSize = 13f
                    setPadding(0, dp(8), 0, dp(8))
                    setOnClickListener {
                        if (d.status == "DOWNLOADING") ShunnoGetEngine.pause(d.id)
                        else if (d.status != "COMPLETED") ShunnoGetEngine.start(this@MainActivity, d) { }
                    }
                }
                box.addView(row)
            }
        }
        scroll.addView(box)
        AlertDialog.Builder(this)
            .setTitle("⚡ Shunno Get Manager")
            .setView(scroll)
            .setPositiveButton("Close", null)
            .show()
    }

    private fun showTabsSwitcherDialog() {
        val titles = tabs.mapIndexed { idx, t ->
            val prefix = if (idx == activeTabIndex) "● " else "○ "
            "$prefix${t.title.take(36)}"
        }.toTypedArray()

        AlertDialog.Builder(this)
            .setTitle("Open Tabs (${tabs.size})")
            .setItems(titles) { _, idx -> switchToTab(idx) }
            .setPositiveButton("+ New Tab") { _, _ -> createNewTab("https://duckduckgo.com", switchTo = true) }
            .setNeutralButton("Close Current") { _, _ ->
                if (tabs.size > 1) {
                    tabs.removeAt(activeTabIndex).webView.destroy()
                    switchToTab((activeTabIndex - 1).coerceAtLeast(0))
                }
            }
            .show()
    }

    private fun showBookmarksDialog() {
        val cur = currentWebView()
        val items = (listOf("➕ Bookmark Current Page") + bookmarks.map { "★ ${it.first}" }).toTypedArray()
        AlertDialog.Builder(this)
            .setTitle("Portable Bookmarks")
            .setItems(items) { _, idx ->
                if (idx == 0 && cur != null) {
                    bookmarks.add(0, (cur.title ?: "Bookmark") to (cur.url ?: ""))
                    saveBookmarks()
                    Toast.makeText(this, "Bookmarked!", Toast.LENGTH_SHORT).show()
                } else if (idx > 0) {
                    navigateTo(bookmarks[idx - 1].second)
                }
            }
            .show()
    }

    private fun loadBookmarks() {
        val file = File(filesDir, "shunno_bookmarks.txt")
        if (!file.exists()) return
        file.readLines().forEach { line ->
            val parts = line.split("|", limit = 2)
            if (parts.size == 2) bookmarks.add(parts[0] to parts[1])
        }
    }

    private fun saveBookmarks() {
        val file = File(filesDir, "shunno_bookmarks.txt")
        file.writeText(bookmarks.joinToString("
") { "${it.first}|${it.second}" })
    }

    private fun makeIconBtn(label: String, onClick: () -> Unit): TextView =
        TextView(this).apply {
            text = label
            textSize = 19f
            setTypeface(null, Typeface.BOLD)
            setTextColor(Color.parseColor("#0A84FF"))
            gravity = Gravity.CENTER
            layoutParams = LinearLayout.LayoutParams(dp(34), dp(36))
            setOnClickListener { onClick() }
        }

    private fun makeChip(label: String, bgHex: String, fgHex: String, onClick: () -> Unit): TextView =
        TextView(this).apply {
            text = label
            textSize = 12f
            setTypeface(null, Typeface.BOLD)
            setTextColor(Color.parseColor(fgHex))
            background = roundRect(bgHex, 8f)
            setPadding(dp(9), dp(5), dp(9), dp(5))
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                leftMargin = dp(4)
            }
            setOnClickListener { onClick() }
        }

    private fun roundRect(hexColor: String, radiusDp: Float): GradientDrawable =
        GradientDrawable().apply {
            setColor(Color.parseColor(hexColor))
            cornerRadius = dp(radiusDp.toInt()).toFloat()
        }

    private fun dp(value: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), resources.displayMetrics).toInt()
}
