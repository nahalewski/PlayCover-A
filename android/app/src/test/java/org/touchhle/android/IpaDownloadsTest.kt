package org.touchhle.android

import org.junit.Assert.*
import org.junit.Test
import java.io.IOException

class IpaDownloadsTest {
    @Test fun removedInstalledFilesDoNotResurrectAsDownloads() {
        assertFalse(DownloadProtocol.restoreRecord("COMPLETE", false))
        assertTrue(DownloadProtocol.restoreRecord("COMPLETE", true))
        assertTrue(DownloadProtocol.restoreRecord("PAUSED", false))
        assertTrue(DownloadProtocol.restoreRecord("RUNNING", false))
    }
    @Test fun historyPruningOnlyRetiresOldestCompletedMetadata() {
        val jobs = listOf("first" to "COMPLETE", "active" to "RUNNING", "second" to "COMPLETE", "paused" to "PAUSED")
        assertEquals(listOf("first"), DownloadProtocol.historyToRetire(jobs, 4))
        assertEquals(listOf("first", "second"), DownloadProtocol.historyToRetire(jobs, 3))
        assertEquals(emptyList<String>(), DownloadProtocol.historyToRetire(jobs, 5))
        assertEquals(emptyList<String>(), DownloadProtocol.historyToRetire(listOf("active" to "RUNNING"), 1))
    }
    @Test fun transientBackoffRemainsBoundedForLongOutages() {
        assertEquals(1000L, DownloadProtocol.retryDelay(0))
        assertEquals(2000L, DownloadProtocol.retryDelay(1))
        assertEquals(60000L, DownloadProtocol.retryDelay(10000))
        assertEquals(1000L, DownloadProtocol.retryDelay(-1))
    }
    @Test fun resumesOnlyExactRemainingRange() {
        assertEquals(DownloadProtocol.Decision(40, 100), DownloadProtocol.response(206, 40, "bytes 40-99/100", 60, "\"v1\"", "\"v1\""))
    }
    @Test fun fullResponseRestartsInsteadOfAppending() {
        assertEquals(DownloadProtocol.Decision(0, 200), DownloadProtocol.response(200, 40, null, 200, "\"old\"", "\"new\""))
    }
    @Test fun rejectsWrongOffsetLengthAndChangedEntity() {
        val bad = listOf("bytes 0-99/100", "bytes 40-98/100", "bytes 40-100/100", "bytes 40-99/*", "bytes 999999999999999999999-99/100")
        for (range in bad) {
            try { DownloadProtocol.response(206, 40, range, 60, "\"v1\"", "\"v1\""); fail(range) } catch (_: IOException) {}
        }
        try { DownloadProtocol.response(206, 40, "bytes 40-99/100", 59, "\"v1\"", "\"v1\""); fail() } catch (_: IOException) {}
        try { DownloadProtocol.response(206, 40, "bytes 40-99/100", 60, "\"v1\"", "\"v2\""); fail() } catch (_: IOException) {}
        try { DownloadProtocol.response(206, 40, "bytes 40-99/100", 60, null, null); fail() } catch (_: IOException) {}
    }
    @Test fun weakEtagsCannotAuthorizeResume() {
        assertNull(DownloadProtocol.validator("W/\"v1\"", null))
        assertEquals("\"v1\"", DownloadProtocol.validator("\"v1\"", null))
        assertEquals("Wed, 21 Oct 2015 07:28:00 GMT", DownloadProtocol.validator(null, "Wed, 21 Oct 2015 07:28:00 GMT"))
        assertNull(DownloadProtocol.validator(null, "arbitrary"))
    }
    @Test fun unsatisfiedRangeRequiresExactNumericTotal() {
        assertEquals(100L, DownloadProtocol.unsatisfiedLength("bytes */100"))
        assertNull(DownloadProtocol.unsatisfiedLength("bytes */*"))
        assertNull(DownloadProtocol.unsatisfiedLength("bytes */999999999999999999999999"))
        assertTrue(DownloadProtocol.completeRange(100, 100, "\"v1\"", "\"v1\""))
        assertFalse(DownloadProtocol.completeRange(99, 100, "\"v1\"", "\"v1\""))
        assertFalse(DownloadProtocol.completeRange(100, 100, "\"v1\"", "\"v2\""))
        assertFalse(DownloadProtocol.completeRange(100, 100, null, null))
    }
}
