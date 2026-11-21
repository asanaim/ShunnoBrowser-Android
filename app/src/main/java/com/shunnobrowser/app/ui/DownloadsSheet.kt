package com.shunnobrowser.app.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.shunnobrowser.app.core.RustCoreBridge
import kotlinx.coroutines.delay
import org.json.JSONObject

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadsSheet(onDismiss: () -> Unit) {
    val appleBlue = Color(0xFF0A84FF)
    val appleGreen = Color(0xFF30D158)
    var urlInput by remember { mutableStateOf("") }
    var tasks by remember { mutableStateOf<List<JSONObject>>(emptyList()) }

    LaunchedEffect(Unit) {
        while (true) {
            tasks = RustCoreBridge.parseDownloads()
            delay(500)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        shape = RoundedCornerShape(topStart = 22.dp, topEnd = 22.dp),
        containerColor = MaterialTheme.colorScheme.background
    ) {
        Column(modifier = Modifier.fillMaxWidth().heightIn(min = 420.dp, max = 640.dp).padding(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("⚡ Multi-Thread Downloads", fontSize = 20.sp, fontWeight = FontWeight.Bold)
                TextButton(onClick = {
                    RustCoreBridge.nativeClearFinishedDownloads()
                    tasks = RustCoreBridge.parseDownloads()
                }) {
                    Text("🧹 Clean Done", fontSize = 12.sp, color = appleBlue)
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                OutlinedTextField(
                    value = urlInput,
                    onValueChange = { urlInput = it },
                    placeholder = { Text("Paste direct URL, FLAC, MP4, or .m3u8...", fontSize = 13.sp) },
                    singleLine = true,
                    modifier = Modifier.weight(1f)
                )
                Button(
                    onClick = {
                        if (urlInput.isNotBlank()) {
                            RustCoreBridge.nativeEnqueueDownload(urlInput.trim(), 16, "", "", "CHECK", "")
                            urlInput = ""
                            tasks = RustCoreBridge.parseDownloads()
                        }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = appleBlue)
                ) {
                    Text("Get")
                }
            }

            Spacer(modifier = Modifier.height(12.dp))

            if (tasks.isEmpty()) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text("No downloads yet. Saved to Downloads/ShunnoBrowser.", color = Color.Gray)
                }
            } else {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    items(tasks, key = { it.optLong("id") }) { task ->
                        val id = task.optLong("id")
                        val name = task.optString("file_name", "download.bin")
                        val status = task.optString("status", "QUEUED")
                        val downloaded = task.optLong("downloaded_bytes", 0L)
                        val total = task.optLong("total_bytes", 0L)
                        val threads = task.optInt("threads", 1)
                        val progress = if (total > 0) (downloaded.toFloat() / total.toFloat()).coerceIn(0f, 1f) else 0f

                        Card(
                            shape = RoundedCornerShape(14.dp),
                            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Column(modifier = Modifier.padding(14.dp)) {
                                Text(name, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(
                                    text = "$status • $threads Threads • ${downloaded / 1024} KB" +
                                        if (total > 0) " / ${total / 1024} KB" else "",
                                    fontSize = 12.sp,
                                    color = if (status == "COMPLETED") appleGreen else Color.Gray
                                )
                                Spacer(modifier = Modifier.height(6.dp))
                                LinearProgressIndicator(
                                    progress = { progress },
                                    modifier = Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(3.dp)),
                                    color = if (status == "COMPLETED") appleGreen else appleBlue
                                )
                                Spacer(modifier = Modifier.height(6.dp))
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    if (status == "DOWNLOADING") {
                                        OutlinedButton(onClick = { RustCoreBridge.nativePauseDownload(id) }) {
                                            Text("Pause", fontSize = 12.sp)
                                        }
                                    } else if (status != "COMPLETED") {
                                        OutlinedButton(onClick = { RustCoreBridge.nativeResumeDownload(id, "") }) {
                                            Text("Resume", fontSize = 12.sp)
                                        }
                                    }
                                    TextButton(onClick = { RustCoreBridge.nativeDeleteDownload(id) }) {
                                        Text("Delete", fontSize = 12.sp, color = Color(0xFFFF375F))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
