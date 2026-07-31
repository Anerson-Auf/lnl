package su.yufu.lnl

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import su.yufu.lnl.ui.LnlApp
import su.yufu.lnl.ui.theme.LnlAppTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            LnlAppTheme {
                LnlApp()
            }
        }
    }
}
