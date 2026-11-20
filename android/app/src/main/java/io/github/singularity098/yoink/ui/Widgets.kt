package io.github.singularity098.yoink.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

val Radius = RoundedCornerShape(12.dp)

/** A translucent black box with a hairline outline: the base of every panel and row. */
@Composable
fun Panel(
    modifier: Modifier = Modifier,
    popup: Boolean = false,
    padding: PaddingValues = PaddingValues(14.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier
            .clip(Radius)
            .background(if (popup) Palette.popup else Palette.surface)
            .border(1.dp, Palette.border, Radius)
            .padding(padding),
        content = content,
    )
}

/**
 * The desktop's BigButton: outlined, or filled with the theme's paint
 * ([primary]), or red ([danger]).
 */
@Composable
fun BigButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    primary: Boolean = false,
    danger: Boolean = false,
    enabled: Boolean = true,
) {
    val look = LocalLook.current
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val fg = when {
        danger && enabled -> Color.White
        primary && enabled -> Palette.onPaint
        else -> look.text
    }
    var m = modifier
        .defaultMinSize(minHeight = 44.dp, minWidth = 44.dp)
        .alpha(if (enabled) 1f else 0.45f)
        .clip(Radius)
    m = when {
        danger && enabled -> m.background(if (pressed) Palette.danger.copy(alpha = 0.75f) else Palette.danger)
        primary && enabled -> m.background(look.paint).then(
            if (pressed) Modifier.background(Color(0x40000000)) else Modifier
        )
        else -> m.background(if (pressed) Palette.pressed else Palette.surface).border(1.dp, Palette.borderStrong, Radius)
    }
    Row(
        m.clickable(interactionSource = source, indication = null, enabled = enabled, onClick = onClick)
            .padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
    ) {
        if (icon != null) Icon(icon, null, tint = fg, modifier = Modifier.size(20.dp))
        if (text.isNotEmpty()) Text(
            text,
            color = fg,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = if (primary || danger) FontWeight.Normal else FontWeight.Light,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A round icon button; [active] paints the icon. */
@Composable
fun RoundIconButton(
    icon: ImageVector,
    description: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    tint: Color? = null,
) {
    val look = LocalLook.current
    Box(
        modifier
            .size(44.dp)
            .clip(CircleShape)
            .alpha(if (enabled) 1f else 0.35f)
            .clickable(enabled = enabled, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, description, tint = tint ?: look.textDim, modifier = Modifier.size(22.dp))
    }
}

/** A chip: outlined, or painted when [selected]. */
@Composable
fun Chip(text: String, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val look = LocalLook.current
    val shape = RoundedCornerShape(50)
    Box(
        modifier
            .clip(shape)
            .then(if (selected) Modifier.background(look.paint) else Modifier.background(Palette.surface).border(1.dp, Palette.borderStrong, shape))
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp),
    ) {
        Text(
            text,
            color = if (selected) Palette.onPaint else look.text,
            style = MaterialTheme.typography.labelMedium,
            maxLines = 1,
        )
    }
}

/** A small status label, coloured by what it says. */
@Composable
fun Badge(text: String, color: Color) {
    Text(
        text,
        color = color,
        style = MaterialTheme.typography.labelSmall,
        modifier = Modifier
            .clip(RoundedCornerShape(50))
            .background(color.copy(alpha = 0.12f))
            .border(1.dp, color.copy(alpha = 0.35f), RoundedCornerShape(50))
            .padding(horizontal = 8.dp, vertical = 2.dp),
    )
}

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text.uppercase(),
        style = MaterialTheme.typography.labelSmall,
        color = LocalLook.current.textDim,
        modifier = modifier.padding(top = 18.dp, bottom = 8.dp),
    )
}

@Composable
fun Field(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    singleLine: Boolean = true,
    visual: VisualTransformation = VisualTransformation.None,
    trailing: @Composable (() -> Unit)? = null,
    keyboardOptions: androidx.compose.foundation.text.KeyboardOptions = androidx.compose.foundation.text.KeyboardOptions.Default,
    keyboardActions: androidx.compose.foundation.text.KeyboardActions = androidx.compose.foundation.text.KeyboardActions.Default,
) {
    val look = LocalLook.current
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = if (placeholder.isEmpty()) null else ({ Text(placeholder, color = Palette.textFaint) }),
        singleLine = singleLine,
        visualTransformation = visual,
        trailingIcon = trailing,
        keyboardOptions = keyboardOptions,
        keyboardActions = keyboardActions,
        shape = Radius,
        textStyle = MaterialTheme.typography.bodyLarge,
        colors = OutlinedTextFieldDefaults.colors(
            focusedBorderColor = look.paintB,
            unfocusedBorderColor = Palette.borderStrong,
            focusedLabelColor = look.paintB,
            unfocusedLabelColor = look.textDim,
            cursorColor = look.accent,
            focusedContainerColor = Palette.surface,
            unfocusedContainerColor = Palette.surface,
        ),
        modifier = modifier.fillMaxWidth(),
    )
}

/** A thin meter in the theme's paint. [fraction] null: indeterminate (a sweeping band). */
@Composable
fun Meter(fraction: Float?, modifier: Modifier = Modifier) {
    val look = LocalLook.current
    if (fraction == null) {
        androidx.compose.material3.LinearProgressIndicator(
            modifier = modifier.fillMaxWidth().clip(RoundedCornerShape(50)),
            color = look.paintB,
            trackColor = Palette.track,
        )
    } else {
        Box(modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(50)).background(Palette.track)) {
            Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).height(4.dp).background(look.paint))
        }
    }
}

@Composable
fun Dim(text: String, modifier: Modifier = Modifier, maxLines: Int = Int.MAX_VALUE, color: Color? = null) {
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = color ?: LocalLook.current.textDim,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}

@Composable
fun RowScope.Spacer1() = androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))

@Composable
fun Tinted(color: Color, content: @Composable () -> Unit) =
    CompositionLocalProvider(LocalContentColor provides color, content = content)

fun formatSize(bytes: Long): String = when {
    bytes < 0 -> "?"
    bytes < 1024 -> "$bytes B"
    bytes < 1024 * 1024 -> "%.0f KB".format(bytes / 1024.0)
    bytes < 1024L * 1024 * 1024 -> "%.1f MB".format(bytes / (1024.0 * 1024))
    else -> "%.2f GB".format(bytes / (1024.0 * 1024 * 1024))
}
