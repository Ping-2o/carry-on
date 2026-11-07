/*
 * AppNav.kt — root scaffold: onboarding gate + 4-tab NavigationBar + NavHost,
 * manual DI (no Hilt, per plan.md) via [AppGraph].
 */
package com.carryon.app.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.carryon.app.demo.DemoRunner
import com.carryon.app.discovery.NsdDiscovery
import com.carryon.app.handoff.ReceiveRunner
import com.carryon.app.ui.screens.DemoScreen
import com.carryon.app.ui.screens.EvidenceScreen
import com.carryon.app.ui.screens.OnboardingScreen
import com.carryon.app.ui.screens.ReceiveScreen
import com.carryon.app.ui.screens.SessionsScreen
import com.carryon.app.ui.screens.TopologyScreen

/** True when the OS animator duration scale is 0 (Reduce Motion equivalent). */
fun isReducedMotion(context: Context): Boolean =
    Settings.Global.getFloat(
        context.contentResolver,
        Settings.Global.ANIMATOR_DURATION_SCALE,
        1f,
    ) == 0f

val LocalReducedMotion = compositionLocalOf { false }

/** Manual dependency graph (plan.md: no Hilt). One instance per process. */
object AppGraph {
    @Volatile private var inited = false

    lateinit var demoRunner: DemoRunner
        private set
    lateinit var receiveRunner: ReceiveRunner
        private set
    lateinit var discovery: NsdDiscovery
        private set
    lateinit var topology: TopologyStore
        private set

    fun init(context: Context) {
        if (inited) return
        synchronized(this) {
            if (inited) return
            val appContext = context.applicationContext
            demoRunner = DemoRunner(
                scope = kotlinx.coroutines.MainScope(),
                baseDir = appContext.cacheDir,
            )
            receiveRunner = ReceiveRunner(
                scope = kotlinx.coroutines.MainScope(),
                context = appContext,
                baseDir = appContext.cacheDir,
            )
            discovery = NsdDiscovery(appContext)
            topology = TopologyStore(appContext)
            inited = true
        }
    }
}

@Composable
fun CarryOnApp() {
    val context = LocalContext.current
    remember { AppGraph.init(context); true }
    val reducedMotion = isReducedMotion(context)
    val onboardingDone = remember {
        context.getSharedPreferences("carryon", Context.MODE_PRIVATE)
            .getBoolean("onboarding.done", false)
    }
    val navController = rememberNavController()

    CompositionLocalProvider(LocalReducedMotion provides reducedMotion) {
        val backStack by navController.currentBackStackEntryAsState()
        val route = backStack?.destination?.route
        val showBars = onboardingDone || (route != null && route != "onboarding")

        Scaffold(
            bottomBar = {
                if (showBars) {
                    NavigationBar {
                        // ASCII glyph icons — on-style and no icon-artifact deps.
                        NavigationBarItem(
                            selected = route == "demo",
                            onClick = { navController.navigateTab("demo") },
                            icon = { Text("▶", style = androidx.compose.material3.MaterialTheme.typography.titleMedium) },
                            label = { Text("Demo") },
                        )
                        NavigationBarItem(
                            selected = route == "receive",
                            onClick = { navController.navigateTab("receive") },
                            icon = { Text("▣", style = androidx.compose.material3.MaterialTheme.typography.titleMedium) },
                            label = { Text("Receive") },
                        )
                        NavigationBarItem(
                            selected = route == "topology",
                            onClick = { navController.navigateTab("topology") },
                            icon = { Text("⌂", style = androidx.compose.material3.MaterialTheme.typography.titleMedium) },
                            label = { Text("Topology") },
                        )
                        NavigationBarItem(
                            selected = route == "sessions",
                            onClick = { navController.navigateTab("sessions") },
                            icon = { Text("⇄", style = androidx.compose.material3.MaterialTheme.typography.titleMedium) },
                            label = { Text("Sessions") },
                        )
                        NavigationBarItem(
                            selected = route == "evidence",
                            onClick = { navController.navigateTab("evidence") },
                            icon = { Text("☰", style = androidx.compose.material3.MaterialTheme.typography.titleMedium) },
                            label = { Text("Evidence") },
                        )
                    }
                }
            },
        ) { padding ->
            NavHost(
                navController = navController,
                startDestination = if (onboardingDone) "demo" else "onboarding",
                modifier = Modifier.padding(padding),
            ) {
                composable("onboarding") {
                    OnboardingScreen(
                        onDone = {
                            context.getSharedPreferences("carryon", Context.MODE_PRIVATE)
                                .edit().putBoolean("onboarding.done", true).apply()
                            navController.navigate("demo") {
                                popUpTo("onboarding") { inclusive = true }
                            }
                        },
                    )
                }
                composable("demo") { DemoScreen() }
                composable("receive") { ReceiveScreen() }
                composable("topology") { TopologyScreen() }
                composable("sessions") { SessionsScreen() }
                composable("evidence") { EvidenceScreen() }
            }
        }
    }
}

/** Tab navigation: single-top, no state loss on re-select. */
private fun NavHostController.navigateTab(route: String) {
    navigate(route) {
        popUpTo(graph.startDestinationId) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}
