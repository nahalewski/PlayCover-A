package org.touchhle.android

import org.junit.Assert.*
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Base64

class IpaInfoMetadataTest {
    private fun thin(wide: Boolean, major: Int): ByteArray {
        val header = if (wide) 32 else 28
        val command = if (wide) 24 else 16
        val bytes = ByteArray(header + command)
        val buffer = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        buffer.putInt(0, if (wide) 0xfeedfacf.toInt() else 0xfeedface.toInt())
        buffer.putInt(4, if (wide) 0x100000c else 12)
        buffer.putInt(12, 2); buffer.putInt(16, 1); buffer.putInt(20, command)
        buffer.putInt(header, if (wide) 0x32 else 0x25); buffer.putInt(header + 4, command)
        if (wide) { buffer.putInt(header + 8, 2); buffer.putInt(header + 12, major shl 16) }
        else buffer.putInt(header + 8, major shl 16)
        return bytes
    }
    private fun metadata(bytes: ByteArray) = IpaInfo.readMachOMetadata { offset, count ->
        require(offset >= 0 && offset + count <= bytes.size)
        bytes.copyOfRange(offset.toInt(), offset.toInt() + count)
    }
    @Test fun plistVersionsAreCanonicalAndNumeric() {
        // XML uses Android's Xml pull-parser and is exercised on device.
        // Keep host tests on the pure version logic and binary plist reader.
        assertEquals("14.2.0", IpaInfo.canonicalIosVersion("014.02.0"))
        val binary = Base64.getDecoder().decode("YnBsaXN0MDDRAQJfEBBNaW5pbXVtT1NWZXJzaW9uWDAxNC4wMi4wCAseAAAAAAAAAQEAAAAAAAAAAwAAAAAAAAAAAAAAAAAAACc=")
        assertEquals("14.2.0", IpaInfo.minimumIosVersionFromPlist(binary))
        assertNull(IpaInfo.canonicalIosVersion("14.0 --headless"))
        assertNull(IpaInfo.canonicalIosVersion("0.1"))
        assertNull(IpaInfo.canonicalIosVersion("14.256"))
        assertEquals("14.0", IpaInfo.highestIosVersion("9.3.5", "14.0"))
    }
    @Test fun selected32BitFatSliceDoesNotInherit64BitRequirement() {
        val arm32 = thin(false, 3); val arm64 = thin(true, 14)
        val fat = ByteArray(320)
        val buffer = ByteBuffer.wrap(fat)
        buffer.putInt(0, 0xcafebabe.toInt()); buffer.putInt(4, 2)
        buffer.putInt(8, 0x100000c); buffer.putInt(16, 256); buffer.putInt(20, arm64.size)
        buffer.putInt(28, 12); buffer.putInt(36, 128); buffer.putInt(40, arm32.size)
        arm32.copyInto(fat, 128); arm64.copyInto(fat, 256)
        assertEquals(IpaInfo.Companion.MachOMetadata("3.0.0", false), metadata(fat))
        assertEquals(IpaInfo.Companion.MachOMetadata("14.0.0", true), metadata(arm64))
    }
    @Test fun truncatedOrOversizedCommandMetadataIsRejected() {
        val bytes = thin(true, 14)
        try { metadata(bytes.copyOf(bytes.size - 1)); fail("Truncated metadata accepted") } catch (_: IllegalArgumentException) {}
        ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).putInt(20, 2 * 1024 * 1024)
        try { metadata(bytes); fail("Oversized commands accepted") } catch (_: IllegalArgumentException) {}
    }
}
