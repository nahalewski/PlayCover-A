/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.content.SharedPreferences
import org.json.JSONObject
import java.io.File

/** Persistent results of the import-time symbol check (see [SymbolIndex]). */
object ScanCache {
    private var file: File? = null
    private var entries = JSONObject()

    @Synchronized
    fun init(dir: File) {
        if (file != null) return
        file = SymbolIndex.cacheFile(dir)
        entries = try { JSONObject(file!!.readText().removePrefix("﻿")) } catch (_: Exception) { JSONObject() }
    }

    @Synchronized
    fun get(key: String): SymbolIndex.Report? =
        entries.optJSONObject(key)?.let { try { SymbolIndex.Report.fromJson(it) } catch (_: Exception) { null } }

    @Synchronized
    fun put(key: String, report: SymbolIndex.Report) {
        // Entries for older versions of the same file are no longer useful.
        val prefix = key.substringBefore('|') + "|"
        entries.keys().asSequence().toList().filter { it.startsWith(prefix) && it != key }.forEach { entries.remove(it) }
        entries.put(key, report.toJson())
        try { file?.writeText(entries.toString()) } catch (_: Exception) { }
    }
}

/**
 * How well each app is known to work, for the badge on its tile.
 *
 * Sources, strongest first: what the user marked by hand, a crash the launcher
 * saw since the app was last started, and `compat.json` (written by
 * tools/regress.ps1 and seeded from GAMES_STATUS.md).
 */
class CompatStore(private val prefs: SharedPreferences, private val dir: File) {
    enum class Status(val label: String, val color: Int) {
        WORKS("Works", 0xFF2E7D32.toInt()),
        PARTIAL("Partial", 0xFFB26A00.toInt()),
        BROKEN("Broken", 0xFFB3261E.toInt()),
        RUNS("Runs", 0xFF386A8F.toInt()),
        CRASHES("Crashes", 0xFFB3261E.toInt()),
        UNTESTED("Untested", 0xFF5F6368.toInt());

        companion object {
            fun parse(text: String?): Status? = values().firstOrNull { it.name.equals(text, true) }
        }
    }

    class Badge(val status: Status, val note: String?, val manual: Boolean)

    private var auto = JSONObject()
    private var autoStamp = -1L

    private fun autoEntries(): JSONObject {
        val f = File(dir, "compat.json")
        val stamp = if (f.isFile) f.lastModified() else 0L
        if (stamp != autoStamp) {
            autoStamp = stamp
            auto = try { JSONObject(f.readText().removePrefix("﻿")) } catch (_: Exception) { JSONObject() }
        }
        return auto
    }

    fun badge(ipaName: String): Badge {
        Status.parse(prefs.getString("compat_manual_$ipaName", null))?.let { return Badge(it, null, true) }
        prefs.getString("compat_crash_$ipaName", null)?.let { return Badge(Status.CRASHES, it, false) }
        val entry = autoEntries().optJSONObject(ipaName)
        val status = Status.parse(entry?.optString("status"))
        if (status != null) return Badge(status, entry?.optString("note")?.takeIf { it.isNotEmpty() }, false)
        return Badge(Status.UNTESTED, null, false)
    }

    fun mark(ipaName: String, status: Status?) {
        prefs.edit().apply {
            if (status == null) remove("compat_manual_$ipaName") else putString("compat_manual_$ipaName", status.name)
        }.apply()
    }

    fun recordCrash(ipaName: String, note: String) = prefs.edit().putString("compat_crash_$ipaName", note).apply()
    fun clearCrash(ipaName: String) = prefs.edit().remove("compat_crash_$ipaName").apply()
}
