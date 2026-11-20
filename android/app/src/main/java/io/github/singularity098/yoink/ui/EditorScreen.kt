package io.github.singularity098.yoink.ui

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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.singularity098.yoink.Editor
import io.github.singularity098.yoink.Pattern
import io.github.singularity098.yoink.YoinkModel

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun EditorScreen(model: YoinkModel, e: Editor, padding: PaddingValues) {
    val look = LocalLook.current
    Column(Modifier.fillMaxSize().padding(padding).imePadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            RoundIconButton(Icons.AutoMirrored.Filled.ArrowBack, "Back", { model.closeEditor() })
            Text(if (e.editingId == null) "Add an app" else "Edit", style = MaterialTheme.typography.titleMedium)
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
        ) {
            Field(
                value = e.repoText,
                onChange = { e.repoText = it },
                label = "GitHub repository",
                placeholder = "owner/name, or its address",
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Search, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onSearch = { model.lookUp(e) }),
                trailing = { RoundIconButton(Icons.Default.Search, "Look up", { model.lookUp(e) }) },
            )

            e.info?.let { info ->
                Spacer(Modifier.height(12.dp))
                Text(info.name, style = MaterialTheme.typography.bodyLarge)
                if (info.description.isNotEmpty()) Dim(info.description)

                SectionTitle("Builds from")
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("Releases", "Pre-releases too", "Actions").forEachIndexed { i, label ->
                        Chip(label, e.channelKind == i, onClick = {
                            if (e.channelKind != i) {
                                e.channelKind = i
                                model.loadOffer(e)
                            }
                        })
                    }
                }
                if (e.channelKind == 2) {
                    if (!model.hasToken) {
                        Spacer(Modifier.height(10.dp))
                        Dim("GitHub only lets Actions builds be downloaded with a token: add one in Options.", color = Palette.warn)
                    }
                    if (info.workflows.isNotEmpty()) {
                        SectionTitle("Workflow")
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Chip("Any", e.workflow.isEmpty(), onClick = { e.workflow = ""; model.loadOffer(e) })
                            info.workflows.forEach { w ->
                                Chip(w, e.workflow == w, onClick = { e.workflow = w; model.loadOffer(e) })
                            }
                        }
                    }
                    Spacer(Modifier.height(12.dp))
                    Field(
                        value = e.branch,
                        onChange = { e.branch = it },
                        label = "Branch",
                        placeholder = "${info.defaultBranch} (the default)",
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, autoCorrectEnabled = false),
                        keyboardActions = KeyboardActions(onDone = { model.loadOffer(e) }),
                    )
                }

                e.offer?.let { offer ->
                    SectionTitle("Files in ${offer.version}")
                    if (offer.note.isNotEmpty()) {
                        Dim(offer.note)
                        Spacer(Modifier.height(8.dp))
                    }
                    Panel(padding = PaddingValues(vertical = 4.dp)) {
                        offer.files.forEach { f ->
                            val picked = e.picked == f.name
                            val apk = f.name.lowercase().endsWith(".apk")
                            Row(
                                Modifier.fillMaxWidth().clickable { e.pick(f.name) }.padding(horizontal = 14.dp, vertical = 10.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Box(
                                    Modifier.size(18.dp).clip(CircleShape)
                                        .then(if (picked) Modifier.background(look.paint) else Modifier.border(1.5.dp, Palette.borderStrong, CircleShape)),
                                )
                                Spacer(Modifier.width(12.dp))
                                Text(
                                    f.name,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = if (apk || e.channelKind == 2) look.text else look.textDim,
                                    modifier = Modifier.weight(1f),
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                                Spacer(Modifier.width(8.dp))
                                Dim(formatSize(f.size))
                            }
                        }
                    }
                    if (e.channelKind == 2) {
                        Spacer(Modifier.height(6.dp))
                        Dim("Actions builds come zipped: Yoink takes the APK out (the one for this phone, if there are several).")
                    }

                    SectionTitle("Pattern")
                    Field(
                        value = e.pattern,
                        onChange = { e.pattern = it },
                        label = "File name, with * for what changes",
                        keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
                    )
                    Spacer(Modifier.height(6.dp))
                    val matching = e.matching
                    Dim(
                        when {
                            e.pattern.isBlank() -> "Pick a file above."
                            matching.isEmpty() -> "Matches nothing in ${offer.version}."
                            matching.size == 1 -> "Matches ${matching[0].name}; the next version's file should match too."
                            else -> "Matches ${matching.size} files; the first is used: ${matching[0].name}"
                        },
                        color = if (matching.isEmpty() && e.pattern.isNotBlank()) Palette.warn else null,
                    )
                }
            }

            if (e.busy) {
                Spacer(Modifier.height(16.dp))
                Meter(null)
            }
            if (e.error.isNotEmpty()) {
                Spacer(Modifier.height(12.dp))
                Dim(e.error, color = Palette.danger)
            }
            Spacer(Modifier.height(24.dp))
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            BigButton("Cancel", onClick = { model.closeEditor() }, modifier = Modifier.weight(1f))
            BigButton(
                if (e.editingId == null) "Add" else "Save",
                primary = true,
                enabled = e.info != null && e.pattern.isNotBlank() && !e.busy &&
                    e.offer?.files?.any { Pattern.matches(e.pattern.trim(), it.name) } == true,
                onClick = { model.saveEditor(e) },
                modifier = Modifier.weight(1f),
            )
        }
    }
}
