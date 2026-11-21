package com.shunnobrowser.app

import android.app.PictureInPictureParams
import android.content.Intent
import android.os.Bundle
import android.util.Rational
import android.webkit.WebView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import com.shunnobrowser.app.core.RustCoreBridge
import com.shunnobrowser.app.media.MediaPlaybackService
import com.shunnobrowser.app.ui.CupertinoBrowserScreen

data class BrowserTabModel(
    val id: Long,
    val title: String = "New Tab",
    val url: String = "https://duckduckgo.com",
    val isIncognito: Boolean = false,
    val isPinned: Boolean = false,
    val isMuted: Boolean = false,
    val groupId: Int? = null,
    val webView: WebView? = null
)

data class TabGroupModel(
    val id: Int,
    val name: String,
    val colorHex: String,
    val collapsed: Boolean = false
)

class MainActivity : ComponentActivity() {
    var autoPipEnabled by mutableStateOf(true)
    var hasActiveVideo by mutableStateOf(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        RustCoreBridge.initialize(applicationContext)

        val initialUrl = intent?.dataString?.takeIf { it.startsWith("http") } ?: "https://duckduckgo.com"

        setContent {
            var forceDark by remember { mutableStateOf(false) }
            val darkTheme = forceDark || isSystemInDarkTheme()
            val scheme = if (darkTheme) {
                darkColorScheme(
                    primary = Color(0xFF0A84FF),
                    background = Color(0xFF000000),
                    surface = Color(0xFF1C1C1E),
                    onSurface = Color.White
                )
            } else {
                lightColorScheme(
                    primary = Color(0xFF0A84FF),
                    background = Color(0xFFF2F2F7),
                    surface = Color(0xFFFFFFFF),
                    onSurface = Color(0xFF1C1C1E)
                )
            }

            MaterialTheme(colorScheme = scheme) {
                CupertinoBrowserScreen(
                    initialUrl = initialUrl,
                    isDark = darkTheme,
                    onToggleDark = { forceDark = !forceDark },
                    onEnterPip = { enterPipNow() },
                    onVideoStateChanged = { playing ->
                        hasActiveVideo = playing
                        if (playing) {
                            startForegroundService(
                                Intent(this, MediaPlaybackService::class.java).putExtra("title", "Playing media in background")
                            )
                        }
                    }
                )
            }
        }
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (autoPipEnabled && hasActiveVideo) {
            enterPipNow()
        }
    }

    fun enterPipNow() {
        val params = PictureInPictureParams.Builder()
            .setAspectRatio(Rational(16, 9))
            .build()
        enterPictureInPictureMode(params)
    }
}
