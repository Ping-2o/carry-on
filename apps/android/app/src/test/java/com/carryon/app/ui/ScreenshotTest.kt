/*
 * ScreenshotTest.kt — renders every production screen (light + dark) on a
 * Pixel 7 profile under Robolectric native graphics and records PNGs with
 * Roborazzi into build/outputs/roborazzi/. CI (ui-screenshots.yml) uploads
 * them and publishes a gallery on the PR. Fixtures only — no JNI, no engine.
 */
package com.carryon.app.ui

import androidx.compose.runtime.Composable
import com.carryon.app.ui.model.UiFixtures
import com.carryon.app.ui.screens.ActivityContent
import com.carryon.app.ui.screens.DetailScaffold
import com.carryon.app.ui.screens.DevicesContent
import com.carryon.app.ui.screens.HomeContent
import com.carryon.app.ui.screens.MoreContent
import com.carryon.app.ui.screens.ReceiveContent
import com.carryon.app.ui.theme.CarryOnTheme
import com.github.takahirom.roborazzi.captureRoboImage
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [34], qualifiers = "w412dp-h915dp-xxhdpi")
class ScreenshotTest {

    private fun shot(name: String, tab: TopDest, content: @Composable () -> Unit) {
        for (dark in listOf(false, true)) {
            val suffix = if (dark) "dark" else "light"
            captureRoboImage("build/outputs/roborazzi/$name-$suffix.png") {
                CarryOnTheme(darkTheme = dark) {
                    AppShell(current = tab, onSelect = {}) { content() }
                }
            }
        }
    }

    @Test fun home() = shot("home", TopDest.Home) {
        HomeContent(UiFixtures.home, onReceive = {}, onDevices = {}, onActivity = {}, onDemo = {})
    }

    @Test fun receiveWaiting() = shot("receive-waiting", TopDest.Receive) {
        ReceiveContent(UiFixtures.receiveWaiting, onAccept = {}, onRearm = {}, onCopyPin = {}, onOpenConsole = {})
    }

    @Test fun receiveDone() = shot("receive-done", TopDest.Receive) {
        ReceiveContent(UiFixtures.receiveDone, onAccept = {}, onRearm = {}, onCopyPin = {}, onOpenConsole = {})
    }

    @Test fun devices() = shot("devices", TopDest.Devices) {
        DevicesContent(UiFixtures.devices, onPair = {}, onRevoke = {})
    }

    @Test fun more() = shot("more", TopDest.More) {
        MoreContent(onOpen = {}, versionName = "0.1.0")
    }

    @Test fun activity() = shot("activity", TopDest.More) {
        DetailScaffold("Activity", onBack = {}) { ActivityContent(UiFixtures.activity) }
    }
}
