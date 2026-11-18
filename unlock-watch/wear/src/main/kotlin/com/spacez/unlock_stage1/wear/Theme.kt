package com.spacez.unlock_stage1.wear

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.wear.compose.material3.ColorScheme
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.MotionScheme
import androidx.wear.compose.material3.dynamicColorScheme

/** Continuity palette (design/tokens.css): shades of blue and navy only. */
object Palette {
    const val NAVY_950 = 0xFF070F1C.toInt()
    const val NAVY_900 = 0xFF0A1424.toInt()
    const val NAVY_850 = 0xFF101E36.toInt()
    const val NAVY_800 = 0xFF172A49.toInt()
    const val NAVY_750 = 0xFF1E3459.toInt()
    const val NAVY_700 = 0xFF223A61.toInt()
    const val BLUE_500 = 0xFF5A77A6.toInt()
    const val PRIMARY = 0xFF5AA2FF.toInt()
    const val PRIMARY_DEEP = 0xFF1F4F94.toInt()
    const val SKY = 0xFF8EC5FF.toInt()
    const val ICE = 0xFFCFE3FF.toInt()
    const val TEXT = 0xFFE9F0FB.toInt()
    const val TEXT_2 = 0xFFB4C3DB.toInt()
}

object C {
    val Navy950 = Color(Palette.NAVY_950)
    val Navy900 = Color(Palette.NAVY_900)
    val Navy850 = Color(Palette.NAVY_850)
    val Navy800 = Color(Palette.NAVY_800)
    val Navy750 = Color(Palette.NAVY_750)
    val Navy700 = Color(Palette.NAVY_700)
    val Steel = Color(Palette.BLUE_500)
    val Blue = Color(Palette.PRIMARY)
    val BlueDeep = Color(Palette.PRIMARY_DEEP)
    val Sky = Color(Palette.SKY)
    val Ice = Color(Palette.ICE)
    val Text = Color(Palette.TEXT)
    val Text2 = Color(Palette.TEXT_2)
}

private val scheme = ColorScheme(
    primary = C.Blue, primaryDim = Color(0xFF3F86E6), primaryContainer = C.BlueDeep,
    onPrimary = Color(0xFF04152E), onPrimaryContainer = Color(0xFFDCEBFF),
    secondary = Color(0xFF9DBBE6), secondaryDim = Color(0xFF7F9DC8), secondaryContainer = C.Navy750,
    onSecondary = C.Navy900, onSecondaryContainer = Color(0xFFD6E4F8),
    tertiary = C.Sky, tertiaryDim = Color(0xFF5FA6E6), tertiaryContainer = Color(0xFF173F66),
    onTertiary = Color(0xFF04152E), onTertiaryContainer = Color(0xFFD8ECFF),
    surfaceContainerLow = Color(0xFF0D1A2F), surfaceContainer = Color(0xFF132542), surfaceContainerHigh = Color(0xFF1A3053),
    onSurface = C.Text, onSurfaceVariant = C.Text2,
    outline = C.Steel, outlineVariant = C.Navy700,
    background = C.Navy950, onBackground = C.Text,
    error = Color(0xFFA9CCFF), errorDim = Color(0xFF8DB4EE), errorContainer = Color(0xFF2A3F66),
    onError = Color(0xFF071830), onErrorContainer = Color(0xFFDCE9FF),
)

/** Seed swatches for the Expressive look (hue in degrees). "watch" = the watch face / phone's dynamic colours. */
enum class Seed(val key: String, val label: String, val hue: Float) {
    BLUE("blue", "Blue", 215f), MINT("mint", "Mint", 160f), PEACH("peach", "Peach", 22f), VIOLET("violet", "Violet", 270f)
}
const val COLOUR_WATCH = "watch"

private fun hsl(h: Float, s: Float, l: Float) = Color.hsl(((h % 360f) + 360f) % 360f, s, l)

/** A full dark Wear scheme derived from one seed hue (used when dynamic colour is off or unavailable). */
fun seedScheme(hue: Float) = ColorScheme(
    primary = hsl(hue, .90f, .78f), primaryDim = hsl(hue, .75f, .66f), primaryContainer = hsl(hue, .50f, .28f),
    onPrimary = hsl(hue, .80f, .11f), onPrimaryContainer = hsl(hue, .90f, .90f),
    secondary = hsl(hue + 18, .45f, .75f), secondaryDim = hsl(hue + 18, .40f, .65f), secondaryContainer = hsl(hue + 18, .30f, .25f),
    onSecondary = hsl(hue, .50f, .10f), onSecondaryContainer = hsl(hue + 18, .50f, .90f),
    tertiary = hsl(hue + 60, .65f, .78f), tertiaryDim = hsl(hue + 60, .55f, .66f), tertiaryContainer = hsl(hue + 60, .40f, .26f),
    onTertiary = hsl(hue + 60, .70f, .11f), onTertiaryContainer = hsl(hue + 60, .70f, .90f),
    surfaceContainerLow = hsl(hue, .30f, .09f), surfaceContainer = hsl(hue, .30f, .13f), surfaceContainerHigh = hsl(hue, .28f, .18f),
    onSurface = hsl(hue, .30f, .93f), onSurfaceVariant = hsl(hue, .20f, .76f),
    outline = hsl(hue, .15f, .55f), outlineVariant = hsl(hue, .20f, .26f),
    background = hsl(hue, .30f, .03f), onBackground = hsl(hue, .30f, .93f),
)

/** True while the Expressive look is on (motion, morphing shapes). Read by the screens. */
val LocalExpressive = androidx.compose.runtime.compositionLocalOf { true }

@Composable
fun ContinuityTheme(expressive: Boolean, colour: String, content: @Composable () -> Unit) {
    val ctx = androidx.compose.ui.platform.LocalContext.current
    val cs = remember(expressive, colour) {
        if (!expressive) scheme
        else if (colour == COLOUR_WATCH) runCatching { dynamicColorScheme(ctx) }.getOrNull() ?: seedScheme(Seed.BLUE.hue)
        else seedScheme((Seed.entries.firstOrNull { it.key == colour } ?: Seed.BLUE).hue)
    }
    androidx.compose.runtime.CompositionLocalProvider(LocalExpressive provides expressive) {
        MaterialTheme(colorScheme = cs, motionScheme = if (expressive) MotionScheme.expressive() else MotionScheme.standard(), content = content)
    }
}

/** Theme-following colours (the old fixed navy palette [C] only backs the Classic scheme). */
object K {
    val primary @Composable get() = MaterialTheme.colorScheme.primary
    val accent @Composable get() = MaterialTheme.colorScheme.tertiary
    val text @Composable get() = MaterialTheme.colorScheme.onSurface
    val text2 @Composable get() = MaterialTheme.colorScheme.onSurfaceVariant
    val outline @Composable get() = MaterialTheme.colorScheme.outline
    val bg @Composable get() = MaterialTheme.colorScheme.background
    val low @Composable get() = MaterialTheme.colorScheme.surfaceContainerLow
    val mid @Composable get() = MaterialTheme.colorScheme.surfaceContainer
    val high @Composable get() = MaterialTheme.colorScheme.surfaceContainerHigh
    val line @Composable get() = MaterialTheme.colorScheme.outlineVariant
}
