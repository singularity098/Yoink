package io.github.singularity098.yoink.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.singularity098.yoink.PlanItem
import io.github.singularity098.yoink.YoinkModel

/**
 * What an update will do, shown before anything is downloaded (as the
 * desktop app's plan): each app, its version now and after, the file and
 * its size, and whether a checksum will be verified. Only Install acts.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PlanSheet(model: YoinkModel, items: List<PlanItem>) {
    val look = LocalLook.current
    ModalBottomSheet(
        onDismissRequest = { model.dismissPlan() },
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = Palette.bg,
        scrimColor = androidx.compose.ui.graphics.Color(0xB3000000),
    ) {
        Column(Modifier.padding(horizontal = 16.dp).navigationBarsPadding()) {
            Text(
                when {
                    items.size > 1 -> "Update ${items.size} apps"
                    items[0].reinstall -> "Reinstall ${items[0].app.name}"
                    model.present[items[0].app.packageName] == null || items[0].app.installed == null -> "Install ${items[0].app.name}"
                    else -> "Update ${items[0].app.name}"
                },
                style = MaterialTheme.typography.titleMedium,
            )
            Spacer(Modifier.height(12.dp))
            Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items.forEach { item ->
                    val now = model.present[item.app.packageName]
                    Panel(Modifier.fillMaxWidth()) {
                        Text(item.app.name, style = MaterialTheme.typography.bodyLarge)
                        val from = item.app.installed?.version?.takeIf { now != null }
                        Dim(if (from != null) "$from → ${item.remote.version}" else "${item.remote.version} (new)", color = look.text)
                        Dim("Downloads ${item.remote.fileName} (${formatSize(item.remote.size)})")
                        Dim(
                            when {
                                item.remote.checksumUrl != null -> "Its SHA-256 is checked against the one published."
                                item.remote.needsToken -> "An Actions build: the APK is taken out of the zip."
                                else -> "No checksum is published for it."
                            }
                        )
                        if (item.app.packageName == model.getApplication<android.app.Application>().packageName) {
                            Dim("This is Yoink itself: it restarts once installed.", color = Palette.warn)
                        }
                        if (item.remote.note.isNotEmpty()) Dim(item.remote.note, color = Palette.warn)
                    }
                }
            }
            Spacer(Modifier.height(10.dp))
            Dim("Android checks the signature and refuses an APK signed by someone else, or an older version.")
            Row(Modifier.fillMaxWidth().padding(vertical = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                BigButton("Cancel", onClick = { model.dismissPlan() }, modifier = Modifier.weight(1f))
                BigButton("Install", primary = true, enabled = model.canInstall, onClick = { model.apply() }, modifier = Modifier.weight(1f))
            }
        }
    }
}
