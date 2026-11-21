package com.shunnobrowser.app.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.shunnobrowser.app.core.RustCoreBridge

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BottomMenuSheet(
    addressBarAtBottom: Boolean,
    isDark: Boolean,
    desktopMode: Boolean,
    videoSpeed: Float,
    onToggleAddressBarPosition: () -> Unit,
    onToggleDark: () -> Unit,
    onToggleDesktopUa: () -> Unit,
    onChangeVideoSpeed: (Float) -> Unit,
    onOpenNewPrivateTab: () -> Unit,
    onOpenFindInPage: () -> Unit,
    onEnterPip: () -> Unit,
    onClearSiteData: () -> Unit,
    onDismiss: () -> Unit
) {
    val appleBlue = Color(0xFF0A84FF)

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        shape = RoundedCornerShape(topStart = 22.dp, topEnd = 22.dp),
        containerColor = MaterialTheme.colorScheme.surface
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Shunno Controls", fontSize = 20.sp, fontWeight = FontWeight.Bold)
                Text(
                    text = RustCoreBridge.nativeBlockerStatus().ifBlank { "Shield Active" },
                    fontSize = 12.sp,
                    color = Color(0xFF30D158)
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Address & Search Bar Position", fontSize = 14.sp, fontWeight = FontWeight.Medium)
                Button(
                    onClick = onToggleAddressBarPosition,
                    colors = ButtonDefaults.buttonColors(containerColor = appleBlue),
                    shape = RoundedCornerShape(10.dp)
                ) {
                    Text(if (addressBarAtBottom) "Bottom Bar" else "Top Bar", fontSize = 13.sp)
                }
            }

            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = onOpenNewPrivateTab, modifier = Modifier.weight(1f)) {
                    Text("🕵 Private Tab", fontSize = 12.sp)
                }
                OutlinedButton(onClick = onOpenFindInPage, modifier = Modifier.weight(1f)) {
                    Text("🔍 Find in Page", fontSize = 12.sp)
                }
            }

            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = onToggleDark, modifier = Modifier.weight(1f)) {
                    Text(if (isDark) "☀️ Light Mode" else "🌙 Dark Mode", fontSize = 12.sp)
                }
                OutlinedButton(onClick = onToggleDesktopUa, modifier = Modifier.weight(1f)) {
                    Text(if (desktopMode) "📱 Mobile Site" else "🖥️ Desktop Site", fontSize = 12.sp)
                }
            }

            Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = onEnterPip, modifier = Modifier.weight(1f)) {
                    Text("📺 Picture-in-Picture", fontSize = 12.sp)
                }
                OutlinedButton(onClick = onClearSiteData, modifier = Modifier.weight(1f)) {
                    Text("🧹 Clear Site Data", fontSize = 12.sp)
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text("Media Speed", fontSize = 14.sp)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    listOf(1.0f, 1.5f, 2.0f, 3.0f).forEach { spd ->
                        FilterChip(
                            selected = videoSpeed == spd,
                            onClick = { onChangeVideoSpeed(spd) },
                            label = { Text("${spd}×", fontSize = 12.sp) }
                        )
                    }
                }
            }
            Spacer(modifier = Modifier.height(16.dp))
        }
    }
}
