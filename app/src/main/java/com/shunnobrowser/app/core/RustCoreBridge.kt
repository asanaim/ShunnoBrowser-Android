package com.shunnobrowser.app.core

import android.content.Context
import android.os.Environment
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

object RustCoreBridge {
    init {
        System.loadLibrary("shunno_core")
    }

    fun initialize(context: Context, profileId: String = "default") {
        val dlDir = File(
            Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS),
            "ShunnoBrowser"
        ).apply { mkdirs() }
        nativeInit(context.filesDir.absolutePath, dlDir.absolutePath, profileId)
    }

    external fun nativeInit(baseDir: String, downloadsDir: String, profileId: String)
    external fun nativeShouldBlock(url: String, sourceUrl: String, reqType: String): Boolean
    external fun nativeBlockerStatus(): String
    external fun nativeCosmeticCss(pageUrl: String): String
    external fun nativeSanitizeUrl(rawInput: String): String
    external fun nativeSearchUrl(query: String): String
    external fun nativeHttpsDecide(tabId: Long, url: String): String
    external fun nativeHttpsNavigationDone(tabId: Long, success: Boolean, cancelled: Boolean): String
    external fun nativeGetSettingsJson(): String
    external fun nativeSaveSettingsJson(jsonStr: String): Boolean
    external fun nativeGetBookmarksJson(): String
    external fun nativeSaveBookmarksJson(jsonStr: String)
    external fun nativeGetHistoryJson(): String
    external fun nativeRecordHistory(title: String, url: String)
    external fun nativeClearHistory()
    external fun nativeGetProfilesJson(): String
    external fun nativeCreateProfile(name: String, color: String): String
    external fun nativeEnqueueDownload(
        url: String,
        threads: Int,
        referer: String,
        cookie: String,
        dupMode: String,
        nameHint: String
    ): String
    external fun nativeGetDownloadsJson(): String
    external fun nativePauseDownload(id: Long)
    external fun nativeResumeDownload(id: Long, newUrl: String)
    external fun nativeDeleteDownload(id: Long)
    external fun nativeClearFinishedDownloads(): Int
    external fun nativeListUserScriptsJson(): String
    external fun nativeLoadApprovedUserScripts(host: String): String
    external fun nativeApproveUserScript(sha256: String, approve: Boolean)

    fun decideHttps(tabId: Long, url: String): Pair<String, String> {
        val obj = JSONObject(nativeHttpsDecide(tabId, url))
        return obj.optString("action", "ALLOW") to obj.optString("target", url)
    }

    fun parseDownloads(): List<JSONObject> {
        val arr = JSONArray(nativeGetDownloadsJson())
        return List(arr.length()) { i -> arr.getJSONObject(i) }
    }
}
