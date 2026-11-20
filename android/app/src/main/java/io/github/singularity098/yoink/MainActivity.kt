package io.github.singularity098.yoink

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Snackbar
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import io.github.singularity098.yoink.ui.AppListScreen
import io.github.singularity098.yoink.ui.EditorScreen
import io.github.singularity098.yoink.ui.LocalLook
import io.github.singularity098.yoink.ui.OptionsScreen
import io.github.singularity098.yoink.ui.Palette
import io.github.singularity098.yoink.ui.PlanSheet
import io.github.singularity098.yoink.ui.YoinkTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val model: YoinkModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Android's install confirmations, shown over Yoink.
        lifecycleScope.launch {
            Installer.confirmations.collect { runCatching { startActivity(it) } }
        }
        if (savedInstanceState == null && model.apps.isNotEmpty()) model.checkAll()

        setContent {
            YoinkTheme(model.theme) {
                val look = LocalLook.current
                var options by rememberSaveable { mutableStateOf(false) }
                val snackbar = remember { SnackbarHostState() }
                LaunchedEffect(model.message) {
                    model.message?.let {
                        snackbar.currentSnackbarData?.dismiss()
                        snackbar.showSnackbar(it)
                        model.message = null
                    }
                }
                val editor = model.editor
                BackHandler(enabled = editor != null || options) {
                    if (editor != null) model.closeEditor() else options = false
                }
                Scaffold(
                    containerColor = Palette.bg,
                    snackbarHost = {
                        SnackbarHost(snackbar) { data ->
                            Snackbar(containerColor = Palette.popup, contentColor = look.text) { Text(data.visuals.message) }
                        }
                    },
                    floatingActionButton = {
                        if (editor == null && !options && model.apps.isNotEmpty()) {
                            FloatingActionButton(
                                onClick = { model.openEditor(null) },
                                containerColor = androidx.compose.ui.graphics.Color.Transparent,
                                contentColor = Palette.onPaint,
                                shape = CircleShape,
                            ) {
                                Box(Modifier.size(56.dp).clip(CircleShape).background(look.paint), contentAlignment = Alignment.Center) {
                                    Icon(Icons.Default.Add, "Add an app")
                                }
                            }
                        }
                    },
                ) { padding ->
                    when {
                        editor != null -> EditorScreen(model, editor, padding)
                        options -> OptionsScreen(model, padding, onBack = { options = false })
                        else -> AppListScreen(
                            model, padding,
                            onOptions = { options = true },
                            onUninstall = { pkg ->
                                runCatching { startActivity(Intent(Intent.ACTION_DELETE, Uri.parse("package:$pkg"))) }
                            },
                        )
                    }
                }
                model.plan?.let { PlanSheet(model, it) }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Installs, uninstalls and the install permission may have changed meanwhile.
        model.refreshPresent()
    }
}
