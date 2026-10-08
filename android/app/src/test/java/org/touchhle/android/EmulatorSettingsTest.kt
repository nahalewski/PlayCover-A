package org.touchhle.android

import org.junit.Assert.assertArrayEquals
import org.junit.Test

class EmulatorSettingsTest {
    @Test fun defaultsPreservePerAppOptions() {
        assertArrayEquals(emptyArray<String>(), EmulatorSettings().runtimeArguments())
    }
    @Test fun overridesProduceSeparateNativeArguments() {
        assertArrayEquals(arrayOf("--scale-hack=3", "--landscape-right",
            "--disable-analog-stick-tilt-controls", "--allow-network-access"),
            EmulatorSettings("3", "landscape-right", false, true).runtimeArguments())
    }
    @Test fun invalidStoredChoicesCannotInjectArguments() {
        assertArrayEquals(emptyArray<String>(),
            EmulatorSettings("8 --headless", "--args").runtimeArguments())
    }
}
