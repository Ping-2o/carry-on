/*
 * MainActivity.kt — single Compose activity. The engine loads with the app
 * (System.loadLibrary in CarryOnFfi) — a load failure surfaces honestly on
 * first engine use, never silently.
 */
package com.carryon.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.carryon.app.ui.CarryOnApp
import com.carryon.app.ui.theme.CarryOnTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            CarryOnTheme {
                CarryOnApp()
            }
        }
    }
}
