package org.touchhle.android

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.util.zip.Deflater

class IpaInfoTest {
    private fun png(width: Int = 1, filter: Int = 0, truncate: Boolean = false): ByteArray {
        val output = ByteArrayOutputStream()
        val stream = DataOutputStream(output)
        stream.write(byteArrayOf(137.toByte(),80,78,71,13,10,26,10))
        fun chunk(type: String, bytes: ByteArray) {
            stream.writeInt(bytes.size); stream.writeBytes(type); stream.write(bytes); stream.writeInt(0)
        }
        chunk("CgBI", ByteArray(4))
        val header = ByteArrayOutputStream()
        DataOutputStream(header).apply { writeInt(width); writeInt(1); write(byteArrayOf(8,6,0,0,0)) }
        chunk("IHDR", header.toByteArray())
        val deflater = Deflater(6, true)
        val compressed = ByteArray(100)
        val size = try {
            deflater.setInput(byteArrayOf(filter.toByte(), 25, 50, 100, 128.toByte())); deflater.finish()
            deflater.deflate(compressed)
        } finally { deflater.end() }
        chunk("IDAT", compressed.copyOf(if (truncate) size - 1 else size))
        chunk("IEND", ByteArray(0))
        return output.toByteArray()
    }

    @Test fun rawDeflatePremultipliedBgraIsConverted() {
        val image = IpaInfo.decodeCgBIPixels(png())!!
        assertEquals(1, image.width); assertEquals(1, image.height)
        assertEquals(0x80c76331.toInt(), image.argb.single())
    }

    @Test fun malformedDimensionsFiltersAndStreamsAreRejected() {
        assertNull(IpaInfo.decodeCgBIPixels(png(width = Int.MAX_VALUE)))
        assertNull(IpaInfo.decodeCgBIPixels(png(filter = 5)))
        assertNull(IpaInfo.decodeCgBIPixels(png(truncate = true)))
        assertNull(IpaInfo.decodeCgBIPixels(png().copyOf(20)))
        assertFalse(IpaInfo.isCgBI(ByteArray(16)))
    }
}
