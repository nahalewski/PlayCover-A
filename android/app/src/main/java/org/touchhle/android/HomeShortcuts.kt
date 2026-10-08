package org.touchhle.android

import android.app.Activity
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.Bitmap
import android.graphics.drawable.Icon
import android.os.Build
import android.widget.Toast
import java.io.File
import java.security.MessageDigest

/** Pinned launch shortcuts refer to a library filename, never an arbitrary path. */
object HomeShortcuts {
    const val EXTRA_FILE = "shortcut_ipa_filename"
    private fun action(activity: Activity) = "${activity.packageName}.OPEN_IPA"

    internal fun libraryFile(directory: File, name: String?): File? {
        if (name.isNullOrBlank() || name == "." || name == ".." ||
            name.any { it == '/' || it == '\\' || it.isISOControl() }) return null
        return try {
            val root = directory.canonicalFile
            val file = File(root, name).canonicalFile
            if (file.parentFile != root) null
            else if (file.isFile && file.name.endsWith(".ipa", true) ||
                file.isDirectory && file.name.endsWith(".app", true)) file else null
        } catch (_: Exception) { null }
    }

    fun consumeIntent(activity: Activity, intent: Intent?, directory: File, launch: (File) -> Unit): Boolean {
        if (intent?.action != action(activity)) return false
        val name = intent.getStringExtra(EXTRA_FILE)
        intent.removeExtra(EXTRA_FILE)
        intent.action = null
        intent.data = null
        activity.intent = intent
        val app = libraryFile(directory, name)
        if (app == null) Toast.makeText(activity, "This IPA is no longer in your library", Toast.LENGTH_LONG).show()
        else launch(app)
        return true
    }

    fun pin(activity: Activity, app: File, title: String, bitmap: Bitmap?) {
        if (Build.VERSION.SDK_INT < 26) {
            Toast.makeText(activity, "Home screen shortcuts require Android 8 or later", Toast.LENGTH_LONG).show()
            return
        }
        val manager = activity.getSystemService(ShortcutManager::class.java)
        if (manager == null || !manager.isRequestPinShortcutSupported) {
            Toast.makeText(activity, "Your home screen does not support pinned shortcuts", Toast.LENGTH_LONG).show()
            return
        }
        val directory = File(activity.getExternalFilesDir(null), "touchHLE_apps")
        if (libraryFile(directory, app.name) != app.canonicalFile) {
            Toast.makeText(activity, "The IPA must be in your installed library", Toast.LENGTH_LONG).show()
            return
        }
        val id = MessageDigest.getInstance("SHA-256").digest(app.name.toByteArray(Charsets.UTF_8))
            .joinToString("") { "%02x".format(it.toInt() and 255) }
        val shortcutIntent = Intent(activity, LauncherActivity::class.java).apply {
            action = action(activity)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            putExtra(EXTRA_FILE, app.name)
        }
        val label = title.filterNot { it.isISOControl() }.take(80).ifBlank { app.name.take(80) }
        val icon = if (bitmap != null) Icon.createWithAdaptiveBitmap(bitmap)
            else Icon.createWithResource(activity, R.mipmap.ic_launcher)
        try {
            val shortcut = ShortcutInfo.Builder(activity, "ipa-$id")
                .setShortLabel(label).setLongLabel(label).setIcon(icon).setIntent(shortcutIntent).build()
            if (manager.pinnedShortcuts.any { it.id == shortcut.id }) {
                manager.enableShortcuts(listOf(shortcut.id))
                manager.updateShortcuts(listOf(shortcut))
                Toast.makeText(activity, "Home screen shortcut updated", Toast.LENGTH_SHORT).show()
            } else {
                val requested = manager.requestPinShortcut(shortcut, null)
                Toast.makeText(activity, if (requested) "Confirm Add on your home screen" else
                    "Your home screen could not add this shortcut", Toast.LENGTH_LONG).show()
            }
        } catch (_: Exception) {
            Toast.makeText(activity, "Could not add a home screen shortcut", Toast.LENGTH_LONG).show()
        }
    }
}
