package com.shunnobrowser.app.media

import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMuxer
import java.io.File
import java.nio.ByteBuffer

object DashMediaMuxer {
    fun muxVideoAndAudio(videoFile: File, audioFile: File, outputFile: File): Boolean {
        val videoExtractor = MediaExtractor()
        val audioExtractor = MediaExtractor()
        var muxer: MediaMuxer? = null
        return try {
            videoExtractor.setDataSource(videoFile.absolutePath)
            audioExtractor.setDataSource(audioFile.absolutePath)

            val videoTrackIdx = findTrack(videoExtractor, "video/")
            val audioTrackIdx = findTrack(audioExtractor, "audio/")
            if (videoTrackIdx < 0 || audioTrackIdx < 0) return false

            videoExtractor.selectTrack(videoTrackIdx)
            audioExtractor.selectTrack(audioTrackIdx)

            muxer = MediaMuxer(outputFile.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
            val outVideoTrack = muxer.addTrack(videoExtractor.getTrackFormat(videoTrackIdx))
            val outAudioTrack = muxer.addTrack(audioExtractor.getTrackFormat(audioTrackIdx))
            muxer.start()

            copySamples(videoExtractor, muxer, outVideoTrack)
            copySamples(audioExtractor, muxer, outAudioTrack)
            true
        } catch (_: Exception) {
            false
        } finally {
            runCatching { muxer?.stop() }
            runCatching { muxer?.release() }
            runCatching { videoExtractor.release() }
            runCatching { audioExtractor.release() }
        }
    }

    private fun findTrack(extractor: MediaExtractor, mimePrefix: String): Int {
        for (i in 0 until extractor.trackCount) {
            val mime = extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME) ?: continue
            if (mime.startsWith(mimePrefix)) return i
        }
        return -1
    }

    private fun copySamples(extractor: MediaExtractor, muxer: MediaMuxer, outTrack: Int) {
        val buffer = ByteBuffer.allocate(1024 * 1024)
        val info = MediaCodec.BufferInfo()
        while (true) {
            val size = extractor.readSampleData(buffer, 0)
            if (size < 0) break
            info.offset = 0
            info.size = size
            info.presentationTimeUs = extractor.sampleTime
            info.flags = extractor.sampleFlags
            muxer.writeSampleData(outTrack, buffer, info)
            extractor.advance()
        }
    }
}
