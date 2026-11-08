/*
 * Components.kt — the Carry-On design system on top of Material 3: cards,
 * status chips, metric tiles, list rows, empty states and the collapsible
 * "Technical details" disclosure that keeps engine plumbing out of the way.
 */
package com.carryon.app.ui.design

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.carryon.app.ui.model.ActivityUi
import com.carryon.app.ui.model.DeviceUi
import com.carryon.app.ui.model.Tone
import com.carryon.app.ui.theme.LocalStatusColors

/** Spacing tokens (4-pt grid). */
object Space {
    val xs = 4.dp
    val s = 8.dp
    val m = 12.dp
    val l = 16.dp
    val xl = 24.dp
    val xxl = 32.dp
}

val ScreenPadding = PaddingValues(horizontal = Space.l, vertical = Space.s)

@Composable
fun Tone.containerColor(): Color = when (this) {
    Tone.Neutral -> MaterialTheme.colorScheme.surfaceContainerHighest
    Tone.Info -> MaterialTheme.colorScheme.secondaryContainer
    Tone.Success -> LocalStatusColors.current.successContainer
    Tone.Warning -> LocalStatusColors.current.warningContainer
    Tone.Danger -> MaterialTheme.colorScheme.errorContainer
}

@Composable
fun Tone.contentColor(): Color = when (this) {
    Tone.Neutral -> MaterialTheme.colorScheme.onSurfaceVariant
    Tone.Info -> MaterialTheme.colorScheme.onSecondaryContainer
    Tone.Success -> LocalStatusColors.current.onSuccessContainer
    Tone.Warning -> LocalStatusColors.current.onWarningContainer
    Tone.Danger -> MaterialTheme.colorScheme.onErrorContainer
}

/** A grouped surface card with an optional header row. */
@Composable
fun SectionCard(
    modifier: Modifier = Modifier,
    title: String? = null,
    icon: ImageVector? = null,
    action: (@Composable () -> Unit)? = null,
    contentPadding: Dp = Space.l,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
    ) {
        Column(Modifier.padding(contentPadding), verticalArrangement = Arrangement.spacedBy(Space.m)) {
            if (title != null) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (icon != null) {
                        Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp))
                        Spacer(Modifier.width(Space.s))
                    }
                    Text(
                        title,
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.weight(1f).semantics { heading() },
                    )
                    action?.invoke()
                }
            }
            content()
        }
    }
}

/** Small tonal pill for a status. */
@Composable
fun StatusChip(text: String, tone: Tone, modifier: Modifier = Modifier, icon: ImageVector? = null) {
    Surface(
        modifier = modifier,
        shape = CircleShape,
        color = tone.containerColor(),
        contentColor = tone.contentColor(),
    ) {
        Row(
            Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            if (icon != null) Icon(icon, contentDescription = null, modifier = Modifier.size(14.dp))
            Text(text, style = MaterialTheme.typography.labelMedium, maxLines = 1)
        }
    }
}

/** A tinted circular glyph for list rows and heroes. */
@Composable
fun IconBadge(icon: ImageVector, tone: Tone = Tone.Info, size: Dp = 40.dp) {
    Box(
        Modifier.size(size).background(tone.containerColor(), CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = tone.contentColor(), modifier = Modifier.size(size * 0.5f))
    }
}

/** A compact metric (label, big value). */
@Composable
fun MetricTile(label: String, value: String, icon: ImageVector, modifier: Modifier = Modifier, tone: Tone = Tone.Info) {
    Surface(
        modifier = modifier,
        shape = MaterialTheme.shapes.medium,
        color = MaterialTheme.colorScheme.surfaceContainer,
    ) {
        Column(Modifier.padding(Space.m), verticalArrangement = Arrangement.spacedBy(Space.xs)) {
            Icon(icon, contentDescription = null, tint = tone.contentColor().copy(alpha = 0.9f), modifier = Modifier.size(18.dp))
            Text(value, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, maxLines = 1)
            Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1)
        }
    }
}

@Composable
fun MetricRow(content: @Composable RowScope.() -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Space.s), content = content)
}

private val TransparentListColors
    @Composable get() = ListItemDefaults.colors(containerColor = Color.Transparent)

@Composable
fun DeviceListItem(
    device: DeviceUi,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = { StatusChip(device.status.label, device.status.tone) },
) {
    ListItem(
        modifier = if (onClick != null) modifier.clickable(role = Role.Button, onClick = onClick) else modifier,
        colors = TransparentListColors,
        leadingContent = { IconBadge(device.kind.icon, device.status.tone) },
        headlineContent = { Text(device.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(device.detail, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = trailing,
    )
}

@Composable
fun ActivityListItem(item: ActivityUi, icon: ImageVector, modifier: Modifier = Modifier) {
    ListItem(
        modifier = modifier,
        colors = TransparentListColors,
        leadingContent = { IconBadge(icon, item.tone) },
        overlineContent = { Text("${item.role.label} · ${item.whenLabel}") },
        headlineContent = { Text(item.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(item.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = { StatusChip(item.stateLabel, item.tone) },
    )
}

/** A navigation row (More screen, settings-style lists). */
@Composable
fun NavRow(title: String, subtitle: String, icon: ImageVector, onClick: () -> Unit, tone: Tone = Tone.Neutral) {
    ListItem(
        modifier = Modifier.clickable(role = Role.Button, onClick = onClick),
        colors = TransparentListColors,
        leadingContent = { IconBadge(icon, tone) },
        headlineContent = { Text(title) },
        supportingContent = { Text(subtitle, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        trailingContent = { Icon(Icons.Outlined.ChevronRight, contentDescription = null) },
    )
}

@Composable
fun ListDivider() {
    HorizontalDivider(Modifier.padding(start = 72.dp), color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f))
}

@Composable
fun EmptyState(icon: ImageVector, title: String, message: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    Column(
        modifier.fillMaxWidth().padding(vertical = Space.xl, horizontal = Space.l),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Space.s),
    ) {
        IconBadge(icon, Tone.Neutral, size = 56.dp)
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        action?.invoke()
    }
}

/** Brand gradient hero container. */
@Composable
fun HeroSurface(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val cs = MaterialTheme.colorScheme
    Box(
        modifier
            .fillMaxWidth()
            .background(
                Brush.linearGradient(listOf(cs.primaryContainer, cs.tertiaryContainer)),
                RoundedCornerShape(28.dp),
            ),
    ) {
        Column(Modifier.padding(Space.xl), verticalArrangement = Arrangement.spacedBy(Space.m), content = content)
    }
}

/** Collapsible developer details; collapsed by default. */
@Composable
fun TechnicalDetails(title: String = "Technical details", content: @Composable ColumnScope.() -> Unit) {
    var open by rememberSaveable { mutableStateOf(false) }
    val angle by animateFloatAsState(if (open) 180f else 0f, label = "chevron")
    SectionCard(contentPadding = 0.dp) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(role = Role.Button) { open = !open }
                .padding(Space.l),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
            Icon(Icons.Outlined.ExpandMore, contentDescription = if (open) "Collapse" else "Expand", modifier = Modifier.rotate(angle))
        }
        AnimatedVisibility(open) {
            Column(
                Modifier.padding(start = Space.l, end = Space.l, bottom = Space.l),
                verticalArrangement = Arrangement.spacedBy(Space.s),
                content = content,
            )
        }
    }
}

@Composable
fun KeyValue(label: String, value: String, mono: Boolean = false) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Space.m)) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.width(120.dp))
        Text(
            value,
            style = if (mono) com.carryon.app.ui.theme.ConsoleTextStyle else MaterialTheme.typography.bodyMedium,
            modifier = Modifier.weight(1f),
        )
    }
}

/** The honesty label every surface carries (spec §2). */
@Composable
fun HonestyLabel(text: String = "Demo · OBSERVED (LOCAL)") {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}
