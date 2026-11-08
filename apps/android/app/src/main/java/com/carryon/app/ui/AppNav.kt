/*
 * AppNav.kt — root: onboarding gate + adaptive navigation (bottom bar on
 * phones, rail on tablets/foldables via NavigationSuiteScaffold) + NavHost,
 * manual DI (no Hilt, per plan.md) via [AppGraph].
 *
 * Four top-level destinations (Home, Receive, Devices, More); every engine /
 * developer surface stays reachable as a pushed route under More.
 */
package com.carryon.app.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffoldDefaults
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteType
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.carryon.app.BuildConfig
import com.carryon.app.demo.DemoRunner
import com.carryon.app.discovery.NsdDiscovery
import com.carryon.app.handoff.ReceiveRunner
import com.carryon.app.kiwix.KiwixRunner
import com.carryon.app.ui.screens.ActivityRoute
import com.carryon.app.ui.screens.DemoScreen
import com.carryon.app.ui.screens.DetailScaffold
import com.carryon.app.ui.screens.DevicesRoute
import com.carryon.app.ui.screens.EvidenceScreen
import com.carryon.app.ui.screens.HomeRoute
import com.carryon.app.ui.screens.KiwixScreen
import com.carryon.app.ui.screens.MoreContent
import com.carryon.app.ui.screens.OnboardingScreen
import com.carryon.app.ui.screens.PairingScreen
import com.carryon.app.ui.screens.ReceiveRoute
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
    lateinit var kiwixRunner: KiwixRunner
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
            topology = TopologyStore(appContext)
            receiveRunner = ReceiveRunner(
                scope = kotlinx.coroutines.MainScope(),
                context = appContext,
                baseDir = appContext.cacheDir,
                topology = topology,
            )
            kiwixRunner = KiwixRunner(
                scope = kotlinx.coroutines.MainScope(),
                context = appContext,
                baseDir = appContext.cacheDir,
                topology = topology,
            )
            discovery = NsdDiscovery(appContext)
            inited = true
        }
    }
}

/** The four top-level destinations. */
enum class TopDest(val route: String, val label: String, val icon: ImageVector, val selectedIcon: ImageVector) {
    Home("home", "Home", Icons.Outlined.Home, Icons.Filled.Home),
    Receive("receive", "Receive", Icons.Outlined.Download, Icons.Filled.Download),
    Devices("devices", "Devices", Icons.Outlined.Devices, Icons.Filled.Devices),
    More("more", "More", Icons.Outlined.MoreHoriz, Icons.Filled.MoreHoriz),
}

/** Pushed routes: route -> app-bar title. All live under the More tab. */
val DetailRoutes: Map<String, String> = linkedMapOf(
    "activity" to "Activity",
    "demo" to "Engine demo",
    "receive/console" to "Receive console",
    "pairing" to "Pairing",
    "topology" to "Topology",
    "sessions" to "Sessions",
    "kiwix" to "Offline library",
    "evidence" to "Evidence",
)

fun topDestFor(route: String?): TopDest? = when {
    route == null || route == "onboarding" -> null
    else -> TopDest.entries.firstOrNull { it.route == route } ?: TopDest.More
}

/**
 * The adaptive app shell (shared by the app and the screenshot tests): a
 * NavigationBar on compact widths, a NavigationRail on medium/expanded.
 */
@Composable
fun AppShell(current: TopDest?, onSelect: (TopDest) -> Unit, content: @Composable () -> Unit) {
    val layoutType = if (current == null) {
        NavigationSuiteType.None
    } else {
        NavigationSuiteScaffoldDefaults.calculateFromAdaptiveInfo(currentWindowAdaptiveInfo())
    }
    NavigationSuiteScaffold(
        layoutType = layoutType,
        containerColor = MaterialTheme.colorScheme.surface,
        navigationSuiteItems = {
            TopDest.entries.forEach { d ->
                item(
                    selected = d == current,
                    onClick = { onSelect(d) },
                    icon = { Icon(if (d == current) d.selectedIcon else d.icon, contentDescription = null) },
                    label = { Text(d.label) },
                )
            }
        },
    ) {
        content()
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
        val current = if (route == null && onboardingDone) TopDest.Home else topDestFor(route)
        val open: (String) -> Unit = { r -> navController.navigate(r) { launchSingleTop = true } }
        val back: () -> Unit = { navController.popBackStack() }

        AppShell(current = current, onSelect = { navController.navigateTab(it.route) }) {
            NavHost(
                navController = navController,
                startDestination = if (onboardingDone) TopDest.Home.route else "onboarding",
            ) {
                composable("onboarding") {
                    OnboardingScreen(
                        onDone = {
                            context.getSharedPreferences("carryon", Context.MODE_PRIVATE)
                                .edit().putBoolean("onboarding.done", true).apply()
                            navController.navigate(TopDest.Home.route) {
                                popUpTo("onboarding") { inclusive = true }
                            }
                        },
                    )
                }
                composable(TopDest.Home.route) { HomeRoute(onNavigate = open) }
                composable(TopDest.Receive.route) { ReceiveRoute(onNavigate = open) }
                composable(TopDest.Devices.route) { DevicesRoute(onNavigate = open) }
                composable(TopDest.More.route) { MoreContent(onOpen = open, versionName = BuildConfig.VERSION_NAME) }

                composable("activity") { DetailScaffold(DetailRoutes.getValue("activity"), back) { ActivityRoute() } }
                composable("demo") { DetailScaffold(DetailRoutes.getValue("demo"), back) { DemoScreen() } }
                composable("receive/console") { DetailScaffold(DetailRoutes.getValue("receive/console"), back) { ReceiveScreen() } }
                composable("pairing") { DetailScaffold(DetailRoutes.getValue("pairing"), back) { PairingScreen() } }
                composable("topology") { DetailScaffold(DetailRoutes.getValue("topology"), back) { TopologyScreen() } }
                composable("sessions") { DetailScaffold(DetailRoutes.getValue("sessions"), back) { SessionsScreen() } }
                composable("kiwix") { DetailScaffold(DetailRoutes.getValue("kiwix"), back) { KiwixScreen() } }
                composable("evidence") { DetailScaffold(DetailRoutes.getValue("evidence"), back) { EvidenceScreen() } }
            }
        }
    }
}

/** Tab navigation: single-top, per-tab state saved and restored. */
private fun NavHostController.navigateTab(route: String) {
    navigate(route) {
        popUpTo(graph.startDestinationId) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}
