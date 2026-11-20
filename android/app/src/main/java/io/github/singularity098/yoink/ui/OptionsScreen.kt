package io.github.singularity098.yoink.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.singularity098.yoink.BuildConfig
import io.github.singularity098.yoink.YoinkModel

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun OptionsScreen(model: YoinkModel, padding: PaddingValues, onBack: () -> Unit) {
    val look = LocalLook.current
    val context = LocalContext.current
    var token by remember { mutableStateOf("") }
    val pickFolder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { tree ->
        if (tree != null) model.setApkFolder(tree)
    }

    Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
        Row(Modifier.fillMaxWidth().padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
            RoundIconButton(Icons.AutoMirrored.Filled.ArrowBack, "Back", onBack)
            Text("Options", style = MaterialTheme.typography.titleMedium)
        }
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            SectionTitle("GitHub token")
            Panel {
                Dim(
                    "Without one, GitHub allows 60 requests an hour and no Actions downloads. A fine-grained " +
                        "token with read access to Contents and Actions (or a classic token with no scopes, for " +
                        "public repositories) is enough. It is kept encrypted with a key in the phone's keystore."
                )
                Spacer(Modifier.height(12.dp))
                if (model.hasToken) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Badge("Token saved", Palette.ok)
                        Spacer1()
                        BigButton("Remove", onClick = { model.setToken(null) })
                    }
                } else {
                    Field(
                        value = token,
                        onChange = { token = it },
                        label = "Token",
                        visual = PasswordVisualTransformation(),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
                    )
                    Spacer(Modifier.height(10.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        BigButton("Create one", onClick = { openUrl(context, "https://github.com/settings/personal-access-tokens/new") })
                        BigButton("Save", primary = true, enabled = token.isNotBlank(), onClick = { model.setToken(token); token = "" })
                    }
                }
            }

            SectionTitle("Actions builds")
            Panel {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Allow failed runs", style = MaterialTheme.typography.bodyMedium)
                        Dim("Take a build from a run that failed (another platform's job, say) when it has the file.")
                    }
                    Spacer(Modifier.width(10.dp))
                    Switch(
                        checked = model.allowFailed,
                        onCheckedChange = { model.changeAllowFailed(it) },
                        colors = SwitchDefaults.colors(
                            checkedTrackColor = look.paintB,
                            checkedThumbColor = Palette.onPaint,
                            uncheckedTrackColor = Palette.track,
                            uncheckedBorderColor = Palette.borderStrong,
                            uncheckedThumbColor = look.textDim,
                        ),
                    )
                }
            }

            SectionTitle("Installing")
            Panel {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("Install unknown apps", style = MaterialTheme.typography.bodyMedium)
                        Dim(if (model.canInstall) "Allowed for Yoink." else "Not allowed yet: Android won't let Yoink install anything.")
                    }
                    Spacer(Modifier.width(10.dp))
                    BigButton(if (model.canInstall) "Settings" else "Allow", primary = !model.canInstall, onClick = { openInstallPermission(context) })
                }
            }

            SectionTitle("Saving APKs")
            Panel {
                Text("APK folder", style = MaterialTheme.typography.bodyMedium)
                val folder = model.apkFolderName
                Dim(
                    when {
                        folder == null -> "Off. Pick a folder to keep a copy of each APK Yoink installs there " +
                            "(to share, sync or back up), each app's new APK replacing its previous one."
                        model.apkFolderLost -> "Yoink lost access to the folder (its storage removed, or access revoked): pick it again."
                        else -> "Each APK Yoink installs is also saved to $folder, replacing that app's previous APK."
                    },
                    color = if (model.apkFolderLost) Palette.warn else null,
                )
                Spacer(Modifier.height(10.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    BigButton(
                        if (folder == null) "Pick a folder" else "Change",
                        primary = folder == null || model.apkFolderLost,
                        onClick = { runCatching { pickFolder.launch(null) } },
                    )
                    if (folder != null) BigButton("Stop saving", onClick = { model.setApkFolder(null) })
                }
            }

            SectionTitle("Theme")
            FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Looks.forEachIndexed { i, l ->
                    val selected = model.theme == i
                    val shape = RoundedCornerShape(12.dp)
                    Column(
                        Modifier.width(96.dp).clip(shape).background(Palette.surface)
                            .border(if (selected) 2.dp else 1.dp, if (selected) l.paintB else Palette.border, shape)
                            .clickable { model.chooseTheme(i) }.padding(10.dp),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Box(Modifier.fillMaxWidth().height(20.dp).clip(RoundedCornerShape(50)).background(l.paint))
                        Spacer(Modifier.height(6.dp))
                        Text(l.name, style = MaterialTheme.typography.labelSmall, color = l.text, maxLines = 1)
                    }
                }
            }

            SectionTitle("Yoink")
            Panel {
                Text("Yoink ${BuildConfig.VERSION_NAME} (build ${BuildConfig.VERSION_CODE})", style = MaterialTheme.typography.bodyMedium)
                if (BuildConfig.COMMIT.isNotEmpty()) Dim("Commit ${BuildConfig.COMMIT.take(7)}")
                Spacer(Modifier.height(8.dp))
                Dim(
                    "Yoink can keep itself up to date: add singularity098/Yoink and pick Yoink-android.apk " +
                        "(from Releases, or the Actions artifact for dev builds)."
                )
                Spacer(Modifier.height(10.dp))
                BigButton("Source code", onClick = { openUrl(context, "https://github.com/singularity098/Yoink") })
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
