package com.shunnobrowser.app

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import java.io.File
import java.io.FileInputStream
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

data class DownloadRecord(
    val id: Long,
    var url: String,
    val fileName: String,
    var totalBytes: Long = -1L,
    var downloadedBytes: Long = 0L,
    var status: String = "DOWNLOADING",
    var speedBps: Long = 0L,
    var threads: Int = 8,
    val category: String = "Downloads",
    var referer: String? = null,
    var cookie: String? = null
)

object ShunnoGetEngine {
    val downloads = mutableListOf<DownloadRecord>()
    private val activeFlags = ConcurrentHashMap<Long, AtomicBoolean>()
    private val pool = Executors.newCachedThreadPool()

    fun detectCategory(fileName: String): String {
        val ext = fileName.substringAfterLast('.', "").lowercase()
        return when (ext) {
            in listOf("mp4", "mkv", "webm", "mov", "ts", "m3u8") -> "Videos"
            in listOf("flac", "alac", "m4a", "wav", "mp3", "ogg", "opus") -> "Lossless Audio"
            in listOf("zip", "rar", "7z", "tar", "gz") -> "Archives"
            "apk" -> "APKs"
            else -> "Documents"
        }
    }

    fun pause(id: Long) {
        activeFlags[id]?.set(false)
    }

    fun start(
        context: Context,
        record: DownloadRecord,
        onUpdate: () -> Unit
    ) {
        pause(record.id)
        val running = AtomicBoolean(true)
        activeFlags[record.id] = running
        record.status = "DOWNLOADING"
        onUpdate()
        BackgroundMediaService.updateOrStart(context, "Downloading ${record.fileName}")

        pool.execute {
            try {
                val partsDir = File(context.cacheDir, "shunnoget_parts").apply { mkdirs() }
                val partFile = File(partsDir, "${record.id}_${record.fileName}.part")

                var contentLength = -1L
                var acceptRanges = false
                try {
                    val headConn = (URL(record.url).openConnection() as HttpURLConnection).apply {
                        requestMethod = "HEAD"
                        connectTimeout = 12000
                        readTimeout = 12000
                        setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 15) ShunnoBrowser/1.0")
                        record.referer?.let { setRequestProperty("Referer", it) }
                        record.cookie?.let { setRequestProperty("Cookie", it) }
                    }
                    contentLength = headConn.contentLengthLong
                    acceptRanges = headConn.getHeaderField("Accept-Ranges")?.equals("bytes", true) == true
                    headConn.disconnect()
                } catch (_: Exception) { }

                record.totalBytes = contentLength
                val threadCount = if (acceptRanges && contentLength > 512 * 1024L) record.threads.coerceIn(1, 16) else 1
                record.threads = threadCount

                if (threadCount > 1 && contentLength > 0L) {
                    RandomAccessFile(partFile, "rw").use { raf ->
                        if (raf.length() != contentLength) raf.setLength(contentLength)
                    }
                    val chunkSize = contentLength / threadCount
                    val totalDone = AtomicLong(0L)
                    val workers = mutableListOf<Thread>()

                    for (t in 0 until threadCount) {
                        val startByte = t * chunkSize
                        val endByte = if (t == threadCount - 1) contentLength - 1 else (startByte + chunkSize - 1)
                        val offsetFile = File(partsDir, "${record.id}_t$t.off")
                        var cur = if (offsetFile.exists()) {
                            offsetFile.readText().trim().toLongOrNull()?.coerceIn(startByte, endByte + 1) ?: startByte
                        } else startByte
                        totalDone.addAndGet(cur - startByte)

                        val worker = Thread {
                            if (cur > endByte) return@Thread
                            val conn = (URL(record.url).openConnection() as HttpURLConnection).apply {
                                setRequestProperty("Range", "bytes=$cur-$endByte")
                                setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 15) ShunnoBrowser/1.0")
                                record.referer?.let { setRequestProperty("Referer", it) }
                                record.cookie?.let { setRequestProperty("Cookie", it) }
                                connectTimeout = 15000
                                readTimeout = 20000
                            }
                            conn.inputStream.use { input ->
                                RandomAccessFile(partFile, "rw").use { raf ->
                                    raf.seek(cur)
                                    val buf = ByteArray(16 * 1024)
                                    var sinceSave = 0
                                    while (running.get()) {
                                        val n = input.read(buf)
                                        if (n == -1) break
                                        raf.write(buf, 0, n)
                                        cur += n
                                        totalDone.addAndGet(n.toLong())
                                        sinceSave += n
                                        if (sinceSave >= 65536) {
                                            offsetFile.writeText(cur.toString())
                                            sinceSave = 0
                                        }
                                    }
                                    offsetFile.writeText(cur.toString())
                                }
                            }
                            conn.disconnect()
                        }
                        workers.add(worker)
                        worker.start()
                    }

                    var lastBytes = totalDone.get()
                    while (workers.any { it.isAlive }) {
                        Thread.sleep(500)
                        val nowBytes = totalDone.get()
                        record.downloadedBytes = nowBytes
                        record.speedBps = ((nowBytes - lastBytes) * 2).coerceAtLeast(0L)
                        lastBytes = nowBytes
                        onUpdate()
                    }
                    workers.forEach { it.join() }

                    if (!running.get()) {
                        record.status = "PAUSED"
                        record.speedBps = 0L
                        onUpdate()
                        return@execute
                    }
                    (0 until threadCount).forEach { File(partsDir, "${record.id}_t$it.off").delete() }
                } else {
                    val conn = (URL(record.url).openConnection() as HttpURLConnection).apply {
                        setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 15) ShunnoBrowser/1.0")
                        record.referer?.let { setRequestProperty("Referer", it) }
                        record.cookie?.let { setRequestProperty("Cookie", it) }
                    }
                    if (record.totalBytes <= 0L) record.totalBytes = conn.contentLengthLong
                    var done = 0L
                    var lastTick = System.currentTimeMillis()
                    var bytesTick = 0L
                    conn.inputStream.use { input ->
                        partFile.outputStream().use { out ->
                            val buf = ByteArray(16 * 1024)
                            while (running.get()) {
                                val n = input.read(buf)
                                if (n == -1) break
                                out.write(buf, 0, n)
                                done += n
                                bytesTick += n
                                val now = System.currentTimeMillis()
                                if (now - lastTick >= 500) {
                                    record.downloadedBytes = done
                                    record.speedBps = (bytesTick * 1000L) / (now - lastTick)
                                    lastTick = now
                                    bytesTick = 0L
                                    onUpdate()
                                }
                            }
                        }
                    }
                    conn.disconnect()
                    if (!running.get()) {
                        record.status = "PAUSED"
                        record.speedBps = 0L
                        onUpdate()
                        return@execute
                    }
                    record.downloadedBytes = done
                    if (record.totalBytes <= 0L) record.totalBytes = done
                }

                exportToPublicDownloads(context, partFile, record.fileName, record.category)
                partFile.delete()
                record.downloadedBytes = record.totalBytes.coerceAtLeast(record.downloadedBytes)
                record.status = "COMPLETED"
                record.speedBps = 0L
                onUpdate()
            } catch (_: Exception) {
                if (running.get()) {
                    record.status = "FAILED"
                    record.speedBps = 0L
                    onUpdate()
                }
            } finally {
                activeFlags.remove(record.id)
            }
        }
    }

    private fun exportToPublicDownloads(context: Context, src: File, name: String, category: String): Uri? {
        if (!src.exists()) return null
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val values = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, name)
                put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/ShunnoBrowser/$category")
            }
            val uri = context.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            if (uri != null) {
                FileInputStream(src).use { input ->
                    context.contentResolver.openOutputStream(uri)?.use { out -> input.copyTo(out) }
                }
            }
            uri
        } else {
            val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "ShunnoBrowser/$category").apply { mkdirs() }
            val dst = File(dir, name)
            src.copyTo(dst, overwrite = true)
            Uri.fromFile(dst)
        }
    }
}
