package com.shunnobrowser.app.ui

import android.annotation.SuppressLint
import android.graphics.Bitmap
import android.view.ViewGroup
import android.webkit.*
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewFeature
import com.shunnobrowser.app.BrowserTabModel
import com.shunnobrowser.app.TabGroupModel
import com.shunnobrowser.app.core.RustCoreBridge
import java.io.ByteArrayInputStream

@SuppressLint("SetJavaScriptEnabled")
@Composable
fun CupertinoBrowserScreen(
    initialUrl: String,
    isDark: Boolean,
    onToggleDark: () -> Unit,
    onEnterPip: () -> Unit,
    onVideoStateChanged: (Boolean) -> Unit
) {
    val context = LocalContext.current
    val focusManager = LocalFocusManager.current
    val appleBlue = Color(0xFF0A84FF)
    val appleGreen = Color(0xFF30D158)

    val contentGuardJs = remember {
        context.assets.open("js/content_guard.js").bufferedReader().use { it.readText() }
    }
    val readabilityJs = remember {
        context.assets.open("js/readability.min.js").bufferedReader().use { it.readText() }
    }
    val readerJs = remember {
        context.assets.open("js/reader.js").bufferedReader().use { it.readText() }
    }

    val tabs = remember {
        mutableStateListOf(BrowserTabModel(id = 1L, title = "DuckDuckGo", url = initialUrl))
    }
    val tabGroups = remember {
        mutableStateListOf(TabGroupModel(id = 1, name = "Research", colorHex = "#0A84FF"))
    }
    var activeTabIndex by remember { mutableIntStateOf(0) }
    var addressBarAtBottom by remember { mutableStateOf(true) }
    var urlInput by remember { mutableStateOf(initialUrl) }
    var pageProgress by remember { mutableFloatStateOf(0f) }
    var detectedMediaUrl by remember { mutableStateOf<String?>(null) }

    var showTabSwitcher by remember { mutableStateOf(false) }
    var showBottomMenu by remember { mutableStateOf(false) }
    var showDownloads by remember { mutableStateOf(false) }
    var showFindBar by remember { mutableStateOf(false) }
    var findQuery by remember { mutableStateOf("") }
    var readerModeActive by remember { mutableStateOf(false) }
    var desktopMode by remember { mutableStateOf(false) }
    var videoSpeed by remember { mutableFloatStateOf(1.0f) }

    val activeTab = tabs.getOrNull(activeTabIndex) ?: tabs.first()

    LaunchedEffect(activeTab.id, activeTab.url) {
        urlInput = activeTab.url
    }

    fun navigateTo(rawInput: String) {
        val sanitized = RustCoreBridge.nativeSanitizeUrl(rawInput)
        val (action, target) = RustCoreBridge.decideHttps(activeTab.id, sanitized)
        val wv = activeTab.webView ?: return
        when (action) {
            "UPGRADE" -> wv.loadUrl(target)
            "INTERSTITIAL" -> wv.loadDataWithBaseURL(sanitized, target, "text/html", "utf-8", null)
            else -> wv.loadUrl(sanitized)
        }
        focusManager.clearFocus()
    }

    fun openNewTab(url: String = "https://duckduckgo.com", incognito: Boolean = false, groupId: Int? = null) {
        val newId = System.currentTimeMillis()
        tabs.add(
            BrowserTabModel(
                id = newId,
                title = if (incognito) "Private Tab" else "New Tab",
                url = url,
                isIncognito = incognito,
                groupId = groupId
            )
        )
        activeTabIndex = tabs.lastIndex
        showTabSwitcher = false
    }

    BackHandler(enabled = activeTab.webView?.canGoBack() == true) {
        activeTab.webView?.goBack()
    }

    @Composable
    fun AddressCapsule() {
        val capsuleBg = if (isDark) Color(0xFF2C2C2E) else Color(0xFFE5E5EA)
        val textColor = if (isDark) Color.White else Color(0xFF1C1C1E)
        val isHttps = activeTab.url.startsWith("https://")

        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(44.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(capsuleBg)
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(
                text = if (activeTab.isIncognito) "🕵️" else if (isHttps) "🔒" else "⚠️",
                fontSize = 13.sp
            )
            BasicTextField(
                value = urlInput,
                onValueChange = { urlInput = it },
                singleLine = true,
                textStyle = TextStyle(color = textColor, fontSize = 14.sp),
                cursorBrush = SolidColor(appleBlue),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { navigateTo(urlInput) }),
                modifier = Modifier.weight(1f),
                decorationBox = { inner ->
                    if (urlInput.isEmpty()) {
                        Text("Search or enter website", color = Color.Gray, fontSize = 14.sp)
                    }
                    inner()
                }
            )
            Text(
                text = "📖",
                fontSize = 15.sp,
                modifier = Modifier.clickable {
                    readerModeActive = !readerModeActive
                    activeTab.webView?.evaluateJavascript(
                        "$readabilityJs\n$readerJs\nif(window.__shunnoToggleReader) window.__shunnoToggleReader();",
                        null
                    )
                }
            )
            Text(
                text = "↻",
                fontSize = 17.sp,
                fontWeight = FontWeight.Bold,
                color = appleBlue,
                modifier = Modifier.clickable { activeTab.webView?.reload() }
            )
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            Column(modifier = Modifier.background(MaterialTheme.colorScheme.surface)) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    tabs.forEachIndexed { idx, tab ->
                        val selected = idx == activeTabIndex
                        val pillBg = when {
                            selected -> appleBlue
                            isDark -> Color(0xFF2C2C2E)
                            else -> Color(0xFFE5E5EA)
                        }
                        Row(
                            modifier = Modifier
                                .clip(RoundedCornerShape(10.dp))
                                .background(pillBg)
                                .clickable { activeTabIndex = idx }
                                .padding(horizontal = 10.dp, vertical = 5.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(6.dp)
                        ) {
                            Text(
                                text = (if (tab.isIncognito) "🕵 " else "") + tab.title,
                                color = if (selected) Color.White else MaterialTheme.colorScheme.onSurface,
                                fontSize = 12.sp,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.widthIn(max = 120.dp)
                            )
                            if (tabs.size > 1) {
                                Text(
                                    text = "✕",
                                    color = if (selected) Color.White.copy(alpha = 0.85f) else Color.Gray,
                                    fontSize = 11.sp,
                                    modifier = Modifier.clickable {
                                        tabs[idx].webView?.destroy()
                                        tabs.removeAt(idx)
                                        activeTabIndex = activeTabIndex.coerceIn(0, tabs.lastIndex)
                                    }
                                )
                            }
                        }
                    }
                    Box(
                        modifier = Modifier
                            .size(26.dp)
                            .clip(CircleShape)
                            .background(if (isDark) Color(0xFF2C2C2E) else Color(0xFFE5E5EA))
                            .clickable { openNewTab() },
                        contentAlignment = Alignment.Center
                    ) {
                        Text("+", fontWeight = FontWeight.Bold, color = appleBlue, fontSize = 15.sp)
                    }
                }

                if (!addressBarAtBottom) {
                    Box(modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) {
                        AddressCapsule()
                    }
                }

                if (showFindBar) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        OutlinedTextField(
                            value = findQuery,
                            onValueChange = {
                                findQuery = it
                                activeTab.webView?.findAllAsync(it)
                            },
                            placeholder = { Text("Find in page...", fontSize = 13.sp) },
                            singleLine = true,
                            modifier = Modifier.weight(1f)
                        )
                        TextButton(onClick = { activeTab.webView?.findNext(false) }) { Text("↑") }
                        TextButton(onClick = { activeTab.webView?.findNext(true) }) { Text("↓") }
                        TextButton(onClick = {
                            activeTab.webView?.clearMatches()
                            showFindBar = false
                        }) { Text("Done") }
                    }
                }

                if (pageProgress in 0.01f..0.99f) {
                    LinearProgressIndicator(
                        progress = { pageProgress },
                        modifier = Modifier.fillMaxWidth().height(2.dp),
                        color = appleBlue
                    )
                }
            }
        },
        bottomBar = {
            Surface(
                color = MaterialTheme.colorScheme.surface.copy(alpha = 0.96f),
                shadowElevation = 8.dp
            ) {
                Column(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                    if (addressBarAtBottom) {
                        AddressCapsule()
                        Spacer(modifier = Modifier.height(8.dp))
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceAround,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        TextButton(onClick = { activeTab.webView?.goBack() }) {
                            Text("‹", fontSize = 24.sp, fontWeight = FontWeight.Bold, color = appleBlue)
                        }
                        TextButton(onClick = { activeTab.webView?.goForward() }) {
                            Text("›", fontSize = 24.sp, fontWeight = FontWeight.Bold, color = appleBlue)
                        }
                        TextButton(onClick = { showDownloads = true }) {
                            Text("⚡", fontSize = 20.sp)
                        }
                        Box(
                            modifier = Modifier
                                .clip(RoundedCornerShape(8.dp))
                                .background(if (isDark) Color(0xFF2C2C2E) else Color(0xFFE5E5EA))
                                .clickable { showTabSwitcher = true }
                                .padding(horizontal = 11.dp, vertical = 4.dp),
                            contentAlignment = Alignment.Center
                        ) {
                            Text(
                                text = "${tabs.size}",
                                fontSize = 13.sp,
                                fontWeight = FontWeight.Bold,
                                color = MaterialTheme.colorScheme.onSurface
                            )
                        }
                        TextButton(onClick = { showBottomMenu = true }) {
                            Text("•••", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = appleBlue)
                        }
                    }
                }
            }
        }
    ) { padding ->
        Box(modifier = Modifier.fillMaxSize().padding(padding)) {
            key(activeTab.id) {
                AndroidView(
                    factory = { ctx ->
                        activeTab.webView ?: WebView(ctx).apply {
                            layoutParams = ViewGroup.LayoutParams(
                                ViewGroup.LayoutParams.MATCH_PARENT,
                                ViewGroup.LayoutParams.MATCH_PARENT
                            )
                            settings.javaScriptEnabled = true
                            settings.domStorageEnabled = true
                            settings.databaseEnabled = true
                            settings.mediaPlaybackRequiresUserGesture = false
                            settings.setSupportZoom(true)
                            settings.builtInZoomControls = true
                            settings.displayZoomControls = false
                            if (WebViewFeature.isFeatureSupported(WebViewFeature.ALGORITHMIC_DARKENING)) {
                                WebSettingsCompat.setAlgorithmicDarkeningAllowed(settings, isDark)
                            }

                            setDownloadListener { url, _, contentDisposition, _, _ ->
                                val cookies = CookieManager.getInstance().getCookie(url) ?: ""
                                RustCoreBridge.nativeEnqueueDownload(
                                    url = url,
                                    threads = 16,
                                    referer = this.url ?: "",
                                    cookie = cookies,
                                    dupMode = "CHECK",
                                    nameHint = contentDisposition ?: ""
                                )
                                showDownloads = true
                            }

                            webChromeClient = object : WebChromeClient() {
                                override fun onProgressChanged(view: WebView?, newProgress: Int) {
                                    pageProgress = newProgress / 100f
                                }

                                override fun onReceivedTitle(view: WebView?, title: String?) {
                                    if (!title.isNullOrBlank()) {
                                        val idx = tabs.indexOfFirst { it.id == activeTab.id }
                                        if (idx != -1) tabs[idx] = tabs[idx].copy(title = title)
                                    }
                                }
                            }

                            webViewClient = object : WebViewClient() {
                                override fun shouldInterceptRequest(
                                    view: WebView?,
                                    request: WebResourceRequest?
                                ): WebResourceResponse? {
                                    val reqUrl = request?.url?.toString() ?: return null
                                    val sourceUrl = view?.url ?: reqUrl
                                    if (RustCoreBridge.nativeShouldBlock(reqUrl, sourceUrl, "other")) {
                                        return WebResourceResponse("text/plain", "utf-8", ByteArrayInputStream(ByteArray(0)))
                                    }
                                    val cleanPath = reqUrl.substringBefore('?').lowercase()
                                    if (cleanPath.endsWith(".mp4") || cleanPath.endsWith(".m3u8") ||
                                        cleanPath.endsWith(".mpd") || cleanPath.endsWith(".flac") ||
                                        cleanPath.endsWith(".mp3") || cleanPath.endsWith(".zip")
                                    ) {
                                        detectedMediaUrl = reqUrl
                                        onVideoStateChanged(true)
                                    }
                                    return super.shouldInterceptRequest(view, request)
                                }

                                override fun onPageStarted(view: WebView?, url: String?, favicon: Bitmap?) {
                                    if (!url.isNullOrBlank()) {
                                        urlInput = url
                                        val idx = tabs.indexOfFirst { it.id == activeTab.id }
                                        if (idx != -1) tabs[idx] = tabs[idx].copy(url = url)
                                    }
                                    view?.evaluateJavascript(contentGuardJs, null)
                                }

                                override fun onPageFinished(view: WebView?, url: String?) {
                                    val u = url ?: return
                                    RustCoreBridge.nativeHttpsNavigationDone(activeTab.id, true, false)
                                    if (!activeTab.isIncognito) {
                                        RustCoreBridge.nativeRecordHistory(view?.title ?: u, u)
                                    }
                                    val css = RustCoreBridge.nativeCosmeticCss(u)
                                    if (css.isNotBlank()) {
                                        val escaped = css.replace("`", "\\`")
                                        view?.evaluateJavascript(
                                            "(function(){var s=document.createElement('style');s.textContent=`$escaped`;document.head.appendChild(s);})();",
                                            null
                                        )
                                    }
                                    val host = android.net.Uri.parse(u).host ?: ""
                                    val userJs = RustCoreBridge.nativeLoadApprovedUserScripts(host)
                                    if (userJs.isNotBlank()) {
                                        view?.evaluateJavascript(userJs, null)
                                    }
                                }
                            }

                            val idx = tabs.indexOfFirst { it.id == activeTab.id }
                            if (idx != -1) tabs[idx] = tabs[idx].copy(webView = this)
                            loadUrl(activeTab.url)
                        }
                    },
                    modifier = Modifier.fillMaxSize()
                )
            }

            detectedMediaUrl?.let { mediaUrl ->
                Surface(
                    color = Color(0xE61C1C1E),
                    shape = RoundedCornerShape(24.dp),
                    modifier = Modifier
                        .align(Alignment.BottomCenter)
                        .padding(14.dp)
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp)
                    ) {
                        Text(
                            text = "0↓ Media Detected",
                            color = Color.White,
                            fontSize = 13.sp,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.weight(1f)
                        )
                        Button(
                            onClick = {
                                val cookies = CookieManager.getInstance().getCookie(mediaUrl) ?: ""
                                RustCoreBridge.nativeEnqueueDownload(
                                    url = mediaUrl,
                                    threads = 16,
                                    referer = activeTab.url,
                                    cookie = cookies,
                                    dupMode = "CHECK",
                                    nameHint = activeTab.title
                                )
                                detectedMediaUrl = null
                                showDownloads = true
                            },
                            colors = ButtonDefaults.buttonColors(containerColor = appleGreen),
                            shape = RoundedCornerShape(16.dp),
                            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 4.dp)
                        ) {
                            Text("Download", color = Color.White, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                        }
                        TextButton(onClick = onEnterPip) {
                            Text("📺 PiP", color = Color.White, fontSize = 12.sp)
                        }
                        Text(
                            text = "✕",
                            color = Color.Gray,
                            fontSize = 14.sp,
                            modifier = Modifier.clickable { detectedMediaUrl = null }
                        )
                    }
                }
            }
        }
    }

    if (showBottomMenu) {
        BottomMenuSheet(
            addressBarAtBottom = addressBarAtBottom,
            isDark = isDark,
            desktopMode = desktopMode,
            videoSpeed = videoSpeed,
            onToggleAddressBarPosition = { addressBarAtBottom = !addressBarAtBottom },
            onToggleDark = onToggleDark,
            onToggleDesktopUa = {
                desktopMode = !desktopMode
                activeTab.webView?.settings?.userAgentString = if (desktopMode) {
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36"
                } else null
                activeTab.webView?.reload()
            },
            onChangeVideoSpeed = { spd ->
                videoSpeed = spd
                activeTab.webView?.evaluateJavascript(
                    "document.querySelectorAll('video,audio').forEach(m=>m.playbackRate=$spd);",
                    null
                )
            },
            onOpenNewPrivateTab = {
                showBottomMenu = false
                openNewTab(incognito = true)
            },
            onOpenFindInPage = {
                showBottomMenu = false
                showFindBar = true
            },
            onEnterPip = {
                showBottomMenu = false
                onEnterPip()
            },
            onClearSiteData = {
                WebStorage.getInstance().deleteAllData()
                CookieManager.getInstance().removeAllCookies(null)
            },
            onDismiss = { showBottomMenu = false }
        )
    }

    if (showTabSwitcher) {
        TabSwitcherSheet(
            tabs = tabs,
            groups = tabGroups,
            activeIndex = activeTabIndex,
            onSelectTab = { idx ->
                activeTabIndex = idx
                showTabSwitcher = false
            },
            onCloseTab = { idx ->
                tabs[idx].webView?.destroy()
                tabs.removeAt(idx)
                if (tabs.isEmpty()) openNewTab()
                activeTabIndex = activeTabIndex.coerceIn(0, tabs.lastIndex)
            },
            onNewTab = { openNewTab() },
            onNewPrivateTab = { openNewTab(incognito = true) },
            onCreateGroup = { name ->
                val nextId = (tabGroups.maxOfOrNull { it.id } ?: 0) + 1
                tabGroups.add(TabGroupModel(id = nextId, name = name, colorHex = "#30D158"))
                tabs[activeTabIndex] = tabs[activeTabIndex].copy(groupId = nextId)
            },
            onDismiss = { showTabSwitcher = false }
        )
    }

    if (showDownloads) {
        DownloadsSheet(onDismiss = { showDownloads = false })
    }
}
