/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. https://mozilla.org/MPL/2.0/ */
package org.touchhle.android

/** Launcher overrides. Defaults deliberately preserve the app's native options. */
internal data class EmulatorSettings(
    val scale: String = "default",
    val orientation: String = "default",
    val controllerTilt: Boolean = true,
    val networkAccess: Boolean = false,
    val autoDownloadRuntime: Boolean = true
) {
    fun runtimeArguments(): Array<String> = buildList {
        if (scale in listOf("1", "2", "3", "4")) add("--scale-hack=$scale")
        when (orientation) {
            "upside-down" -> add("--upside-down")
            "landscape-left" -> add("--landscape-left")
            "landscape-right" -> add("--landscape-right")
        }
        if (!controllerTilt) add("--disable-analog-stick-tilt-controls")
        if (networkAccess) add("--allow-network-access")
    }.toTypedArray()

    companion object {
        const val SCALE = "runtime_scale"
        const val ORIENTATION = "runtime_orientation"
        const val CONTROLLER_TILT = "runtime_controller_tilt"
        const val NETWORK = "runtime_network"
        const val AUTO_DOWNLOAD_RUNTIME = "runtime_auto_download_cache"
    }
}
