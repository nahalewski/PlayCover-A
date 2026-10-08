/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
package org.touchhle.android

import android.util.Log
import java.io.File

/**
 * What the emulator implements (functions, constants, classes), asked of the
 * native library itself so the list can never go stale. Used to guess, before
 * launching, which imports of an app are missing.
 */
object SymbolIndex {
    private external fun exportedSymbols(): String

    @Volatile private var loaded = false
    @Volatile private var failed = false
    private val functions = HashSet<String>()
    private val constants = HashSet<String>()
    private val classes = HashSet<String>()

    /** Incremented when the native library is replaced, so cached scans are not reused. */
    @Volatile var version: String = "0"
        private set

    @Synchronized
    fun ensureLoaded(): Boolean {
        if (loaded) return true
        if (failed) return false
        return try {
            System.loadLibrary("SDL2")
            System.loadLibrary("touchHLE")
            for (line in exportedSymbols().lineSequence()) {
                val bar = line.indexOf('|')
                if (bar != 1) continue
                val name = line.substring(2)
                when (line[0]) {
                    'F' -> functions.add(name)
                    'C' -> constants.add(name)
                    'K' -> classes.add(name)
                }
            }
            version = "${functions.size}-${constants.size}-${classes.size}"
            loaded = true
            true
        } catch (t: Throwable) {
            Log.w("SymbolIndex", "native symbol list unavailable", t)
            failed = true
            false
        }
    }

    /** Result of checking one app's undefined symbols against the emulator. */
    class Report(val missingFunctions: List<String>, val missingConstants: List<String>,
                 val missingClasses: List<String>) {
        /** Missing C functions and data symbols are what crash apps; classes and selectors are tolerated. */
        val serious get() = missingFunctions.size + missingConstants.size
        fun toJson(): org.json.JSONObject = org.json.JSONObject()
            .put("f", org.json.JSONArray(missingFunctions)).put("c", org.json.JSONArray(missingConstants))
            .put("k", org.json.JSONArray(missingClasses))

        companion object {
            fun fromJson(o: org.json.JSONObject): Report {
                fun list(key: String) = o.optJSONArray(key)?.let { a -> (0 until a.length()).map { a.getString(it) } } ?: emptyList()
                return Report(list("f"), list("c"), list("k"))
            }
        }
    }

    // Symbols that are expected not to be in the host table: C++ runtime comes
    // from the app's own libstdc++/libgcc, the rest are linker machinery.
    private fun ignorable(name: String): Boolean =
        name.startsWith("__Z") || name.startsWith("_Z") || name.startsWith("___cxa") || name.startsWith("__cxa") ||
        name.startsWith("__Unwind") || name.startsWith("_Unwind") || name == "dyld_stub_binder" ||
        name.startsWith("_OBJC_METACLASS_\$_") || name.startsWith("_OBJC_EHTYPE_\$_") ||
        name == "___objc_personality_v0" || name.startsWith("__NSConcrete") || name.startsWith("___gxx") ||
        name.startsWith("___gcc") || name.startsWith("__gcc") || name.startsWith("___")

    fun check(undefined: Collection<String>): Report? {
        if (!ensureLoaded()) return null
        val f = ArrayList<String>(); val c = ArrayList<String>(); val k = ArrayList<String>()
        for (name in undefined) {
            if (ignorable(name)) continue
            if (name.startsWith("_OBJC_CLASS_\$_")) {
                if (name.removePrefix("_OBJC_CLASS_\$_") !in classes) k.add(name.removePrefix("_OBJC_CLASS_\$_"))
            } else if (name in functions || name in constants) {
                continue
            } else {
                // Constants are named like types (_NSFoo, _kBar, _AVAudioSession...), functions are lower-case C names.
                val first = name.removePrefix("_").firstOrNull() ?: continue
                if (first.isUpperCase() || name.startsWith("_k")) c.add(name) else f.add(name)
            }
        }
        return Report(f.sorted(), c.sorted(), k.sorted())
    }

    /** Scan results survive between launcher runs, keyed by file identity and emulator build. */
    fun cacheFile(dir: File) = File(dir, "scan_cache.json")
}
