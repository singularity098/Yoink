package io.github.singularity098.yoink.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import io.github.singularity098.yoink.R

/**
 * The desktop app's look (ui/theme.slint, Chippy's): a flat dark
 * background, translucent black boxes with hairline grey outlines, and a
 * theme that paints the highlights with two or three colours.
 */
data class Look(
    val name: String,
    val paintA: Color,
    val paintB: Color,
    val paintC: Color,
    val accent: Color,
    val text: Color,
    val textDim: Color,
) {
    /** The highlight paint: fills of things that are on or picked. */
    val paint: Brush
        get() = Brush.linearGradient(
            0f to paintA, 0.55f to paintB, 1f to paintC,
            start = Offset.Zero, end = Offset(Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY),
        )
}

private fun c(hex: Long) = Color(0xFF000000 or hex)

/** The same looks, in the same order, as the desktop app. Iris is the default. */
val Looks = listOf(
    Look("Iris", c(0x7278fe), c(0xc688d6), c(0xa85eea), c(0xffffff), c(0xe6e6ea), c(0x8a8a94)),
    Look("Ember", c(0xffb36b), c(0xff6a3d), c(0xe0336f), c(0xffb36b), c(0xf0e2d8), c(0x9c8a80)),
    Look("Abyss", c(0x7ff5e0), c(0x2ec8e8), c(0x3a7bff), c(0x7ff5e0), c(0xd8eaf0), c(0x7f99a5)),
    Look("Phosphor", c(0xb8ffc8), c(0x4dff88), c(0x22d36a), c(0xb8ffc8), c(0x9cf0b0), c(0x5f9c6c)),
    Look("Synthwave", c(0xff4fd8), c(0xb65cff), c(0x29e0ff), c(0xff4fd8), c(0xece2fa), c(0x9486ab)),
    Look("Gold Leaf", c(0xf6e2b3), c(0xd4af6a), c(0xa77c3c), c(0xf6e2b3), c(0xece4d4), c(0x948a76)),
    Look("Aurora", c(0x8dffc3), c(0x2fd6c3), c(0x9a6bff), c(0x8dffc3), c(0xdcefe8), c(0x82988f)),
    Look("Blood Moon", c(0xff9aa4), c(0xe0263f), c(0xaf3254), c(0xff9aa4), c(0xeedcde), c(0x9b8285)),
    Look("Glacier", c(0xffffff), c(0xcfe6ff), c(0x8fbaf0), c(0xcfe6ff), c(0xe6eef7), c(0x8796a8)),
    Look("Graphite", c(0xffffff), c(0xd0d0d0), c(0x8e8e8e), c(0xffffff), c(0xe8e8e8), c(0x8e8e8e)),
    Look("Oxide", c(0xf0a46c), c(0xc8693c), c(0x4fc2a6), c(0xf0a46c), c(0xeee2d8), c(0x978a80)),
)

object Palette {
    val bg = Color(0xFF15151A)
    val surface = Color(0x80000000)
    val popup = Color(0xF2000000)
    val surface2 = Color(0x8C000000)
    val pressed = Color(0x1FFFFFFF)
    val track = Color(0x14FFFFFF)
    val border = Color(0x1AFFFFFF)
    val borderStrong = Color(0x29FFFFFF)
    val textFaint = Color(0xFF5C5C64)
    val onPaint = Color.Black
    val ok = Color(0xFF4CC97A)
    val warn = Color(0xFFF0C040)
    val danger = Color(0xFFE5484D)
}

val Jost = FontFamily(
    Font(R.font.jost_light, FontWeight.Light),
    Font(R.font.jost_regular, FontWeight.Normal),
)

val LocalLook = staticCompositionLocalOf { Looks[0] }

@Composable
fun YoinkTheme(index: Int, content: @Composable () -> Unit) {
    val look = Looks.getOrElse(index) { Looks[0] }
    val scheme = darkColorScheme(
        primary = look.paintB,
        onPrimary = Palette.onPaint,
        secondary = look.paintA,
        background = Palette.bg,
        surface = Color(0xFF0C0C10),
        surfaceContainer = Color(0xFF0C0C10),
        surfaceContainerHigh = Color(0xFF101014),
        surfaceContainerHighest = Color(0xFF141418),
        onSurface = look.text,
        onSurfaceVariant = look.textDim,
        outline = Palette.borderStrong,
        outlineVariant = Palette.border,
        error = Palette.danger,
    )
    // Jost runs small, so its sizes are a little larger than usual.
    val base = TextStyle(fontFamily = Jost, fontWeight = FontWeight.Light, color = look.text)
    val type = Typography(
        titleLarge = base.copy(fontSize = 26.sp),
        titleMedium = base.copy(fontSize = 20.sp),
        bodyLarge = base.copy(fontSize = 17.sp),
        bodyMedium = base.copy(fontSize = 16.sp),
        bodySmall = base.copy(fontSize = 14.sp, fontWeight = FontWeight.Normal, color = look.textDim),
        labelLarge = base.copy(fontSize = 16.sp, fontWeight = FontWeight.Normal),
        labelMedium = base.copy(fontSize = 14.sp, fontWeight = FontWeight.Normal),
        labelSmall = base.copy(fontSize = 12.sp, fontWeight = FontWeight.Normal),
    )
    CompositionLocalProvider(LocalLook provides look) {
        MaterialTheme(colorScheme = scheme, typography = type, content = content)
    }
}
