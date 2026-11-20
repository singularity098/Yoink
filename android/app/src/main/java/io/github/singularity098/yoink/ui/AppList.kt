package io.github.singularity098.yoink.ui

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.net.Uri
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.singularity098.yoink.AppEntry
import io.github.singularity098.yoink.Status
import io.github.singularity098.yoink.YoinkModel

@Composable
fun AppListScreen(model: YoinkModel, padding: PaddingValues, onOptions: () -> Unit, onUninstall: (String) -> Unit) {
    val look = LocalLook.current
    val context = LocalContext.current
    var removing by remember { mutableStateOf<AppEntry?>(null) }

    Column(Modifier.fillMaxSize().padding(padding)) {
        // The title bar.
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = 6.dp, top = 6.dp, bottom = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Yoink", style = MaterialTheme.typography.titleLarge)
            Spacer1()
            RoundIconButton(Icons.Default.Refresh, "Check for updates", { model.checkAll() }, enabled = !model.checking && model.apps.isNotEmpty())
            RoundIconButton(Icons.Default.Settings, "Options", onOptions)
        }

        if (!model.canInstall) {
            Panel(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    androidx.compose.material3.Icon(Icons.Default.Warning, null, tint = Palette.warn, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(10.dp))
                    Text("Yoink needs your permission to install apps.", style = MaterialTheme.typography.bodyMedium)
                }
                Spacer(Modifier.height(10.dp))
                BigButton("Allow", primary = true, onClick = { openInstallPermission(context) })
            }
        }

        val updates = model.updatable
        if (updates.size > 1 || model.checking) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    if (model.checking) "Checking for updates…" else "${updates.size} updates",
                    style = MaterialTheme.typography.bodyMedium,
                    color = look.textDim,
                )
                Spacer1()
                if (updates.size > 1) BigButton(
                    "Update all", primary = true, enabled = !model.planning,
                    onClick = { model.prepare(updates.map { it.id }) },
                )
            }
        }

        PullToRefreshBox(
            isRefreshing = model.checking,
            onRefresh = { model.checkAll() },
            modifier = Modifier.weight(1f).fillMaxWidth(),
        ) {
            if (model.apps.isEmpty()) {
                Column(
                    Modifier.fillMaxSize().padding(32.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        "Add an app by its GitHub repository: Yoink fetches its newest APK from Releases or from Actions builds, installs it, and keeps it up to date.",
                        style = MaterialTheme.typography.bodyLarge,
                        color = look.textDim,
                        textAlign = TextAlign.Center,
                    )
                    Spacer(Modifier.height(20.dp))
                    BigButton("Add an app", primary = true, onClick = { model.openEditor(null) })
                }
            } else {
                LazyColumn(
                    contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 6.dp, bottom = 96.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                    modifier = Modifier.fillMaxSize(),
                ) {
                    items(model.apps, key = { it.id }) { app ->
                        AppRow(model, app, onRemove = { removing = app })
                    }
                }
            }
        }
    }

    removing?.let { app ->
        val installed = app.packageName.isNotEmpty() && model.present[app.packageName] != null
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove ${app.name}?") },
            text = {
                Text(
                    if (installed) "Uninstall it from this phone (Android asks first), or only stop Yoink from managing it and leave it installed."
                    else "Yoink will stop managing it.",
                    style = MaterialTheme.typography.bodyMedium,
                )
            },
            confirmButton = {
                if (installed) TextButton(onClick = {
                    removing = null
                    model.beginUninstall(app.id)?.let(onUninstall)
                }) { Text("Uninstall", color = Palette.danger) }
            },
            dismissButton = {
                Row {
                    TextButton(onClick = { removing = null; model.stopManaging(app.id) }) {
                        Text(if (installed) "Stop managing" else "Remove", color = look.text)
                    }
                    TextButton(onClick = { removing = null }) { Text("Cancel", color = look.textDim) }
                }
            },
            containerColor = Palette.bg,
        )
    }
}

@Composable
private fun AppRow(model: YoinkModel, app: AppEntry, onRemove: () -> Unit) {
    val look = LocalLook.current
    val context = LocalContext.current
    val busy = model.busy[app.id]
    val status = model.status(app)
    val present = model.present[app.packageName]
    var menu by remember { mutableStateOf(false) }

    Panel(padding = PaddingValues(start = 12.dp, top = 12.dp, bottom = 12.dp, end = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            AppIcon(model, app, present != null)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(app.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Dim("${app.source} · ${app.channel.label}", maxLines = 1)
                Spacer(Modifier.height(4.dp))
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    when (status) {
                        Status.NotInstalled -> Badge("Not installed", look.textDim)
                        Status.UpdateAvailable -> Badge("Update: ${app.latest?.version}", look.accent)
                        Status.UpToDate -> Badge("Up to date", Palette.ok)
                        Status.Unknown -> Badge("Not checked", look.textDim)
                    }
                    if (present != null) Dim(present.versionName.ifEmpty { "?" } + (app.installed?.version?.let { " · $it" } ?: ""), maxLines = 1)
                }
            }
            when {
                busy != null -> {}
                status == Status.UpdateAvailable ->
                    BigButton("Update", primary = true, enabled = !model.planning, onClick = { model.prepare(listOf(app.id)) })
                status == Status.NotInstalled && app.latest != null ->
                    BigButton("Install", enabled = !model.planning, onClick = { model.prepare(listOf(app.id)) })
            }
            Box {
                RoundIconButton(Icons.Default.MoreVert, "More", { menu = true })
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }, containerColor = Palette.bg) {
                    if (present != null) {
                        context.packageManager.getLaunchIntentForPackage(app.packageName)?.let { launch ->
                            DropdownMenuItem(text = { Text("Open") }, onClick = { menu = false; context.startActivity(launch) })
                        }
                    }
                    DropdownMenuItem(text = { Text("Check for updates") }, onClick = { menu = false; model.checkOne(app.id) })
                    DropdownMenuItem(
                        text = { Text(if (present != null) "Force update (reinstall)" else "Install") },
                        onClick = { menu = false; model.prepare(listOf(app.id), reinstall = present != null) },
                    )
                    DropdownMenuItem(text = { Text("Edit…") }, onClick = { menu = false; model.openEditor(app.id) })
                    app.latest?.web?.takeIf { it.isNotEmpty() }?.let { web ->
                        DropdownMenuItem(text = { Text("Release page") }, onClick = { menu = false; openUrl(context, web) })
                    }
                    DropdownMenuItem(text = { Text("Repository") }, onClick = { menu = false; openUrl(context, app.web) })
                    DropdownMenuItem(text = { Text("Remove…", color = Palette.danger) }, onClick = { menu = false; onRemove() })
                }
            }
        }
        if (busy != null) {
            Spacer(Modifier.height(10.dp))
            Row(Modifier.padding(end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    val label = if (busy.total > 0) "${busy.label} · ${formatSize(busy.done)} of ${formatSize(busy.total)}" else busy.label
                    Dim(label, maxLines = 1)
                    Spacer(Modifier.height(6.dp))
                    Meter(if (busy.total > 0) busy.done.toFloat() / busy.total else null)
                }
                if (busy.cancellable) RoundIconButton(Icons.Default.Close, "Cancel", { model.cancel(app.id) })
            }
        }
        val note = app.latest?.note.orEmpty()
        if (app.error.isNotEmpty() && busy == null) {
            Spacer(Modifier.height(8.dp))
            Dim(app.error, color = Palette.danger, modifier = Modifier.padding(end = 8.dp))
        } else if (note.isNotEmpty() && busy == null && status != Status.UpToDate) {
            Spacer(Modifier.height(8.dp))
            Dim(note, modifier = Modifier.padding(end = 8.dp))
        }
    }
}

/** The installed app's own icon, or the repository owner's avatar until then. */
@Composable
private fun AppIcon(model: YoinkModel, app: AppEntry, installed: Boolean) {
    val context = LocalContext.current
    val iconFile = model.iconFile(app.id)
    val bitmap: ImageBitmap? = remember(app.packageName, installed, iconFile.exists(), model.apps) {
        val fromPackage = if (installed) runCatching {
            drawableBitmap(context.packageManager.getApplicationIcon(app.packageName), 144)
        }.getOrNull() else null
        fromPackage ?: runCatching { BitmapFactory.decodeFile(iconFile.path)?.asImageBitmap() }.getOrNull()
    }
    val shape = RoundedCornerShape(12.dp)
    if (bitmap != null) {
        Image(bitmap, null, Modifier.size(48.dp).clip(shape))
    } else {
        Box(
            Modifier.size(48.dp).clip(shape).background(Palette.surface).border(1.dp, Palette.borderStrong, shape),
            contentAlignment = Alignment.Center,
        ) {
            Text(app.name.take(1).uppercase(), style = MaterialTheme.typography.titleMedium, color = LocalLook.current.paintB)
        }
    }
}

private fun drawableBitmap(d: android.graphics.drawable.Drawable, size: Int): ImageBitmap {
    val b = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
    d.setBounds(0, 0, size, size)
    d.draw(Canvas(b))
    return b.asImageBitmap()
}

fun openUrl(context: Context, url: String) {
    runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}

fun openInstallPermission(context: Context) {
    runCatching {
        context.startActivity(
            Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
        )
    }
}
