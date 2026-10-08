package org.touchhle.android

import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

class RepoSourcesTest {
    @Test fun redirectsIgnoreDocumentFragmentsAndPreserveHttpsValidation() {
        assertEquals("https://cdn.example.com/catalog.json?raw=1", RepoSources.redirectURL(
            "https://example.com/source", "https://cdn.example.com/catalog.json?raw=1#"))
        for (location in listOf("http://example.com/feed", "https://user:password@example.com/feed")) {
            try { RepoSources.redirectURL("https://example.com/source", location); fail("Unsafe redirect accepted") }
            catch (_: IllegalArgumentException) { }
        }
    }
    @Test fun feedByteLimitCountsUtf8WithoutDuplicatingLargeFeeds() {
        assertEquals(32 * 1024 * 1024, RepoSources.FEED_LIMIT)
        assertTrue(RepoSources.feedFits("abcd", 4))
        assertFalse(RepoSources.feedFits("abcd", 3))
        assertTrue(RepoSources.feedFits("é😀", 6))
        assertFalse(RepoSources.feedFits("é😀", 5))
        assertTrue(RepoSources.feedFits("\uD800", 1))
    }
    @Test fun officialFlycastFeedIsAnEmulator() {
        // Published by https://flyinghead.github.io/flycast-builds/altstore.json.
        val feed = RepoSources.parseFeed("""{"name":"Flyinghead","identifier":"com.flyinghead.source","apps":[{
          "name":"Flycast","bundleIdentifier":"com.flyinghead.Flycast","developerName":"Flyinghead",
          "subtitle":"Dreamcast, Naomi and Atomiswave emulator.","version":"v2.5",
          "downloadURL":"https://github.com/flyinghead/flycast/releases/download/v2.5/Flycast-2.5.ipa",
          "localizedDescription":"Flycast is a Dreamcast, Naomi, Naomi 2 and Atomiswave emulator.",
          "iconURL":"https://github.com/flyinghead/flycast/raw/master/shell/linux/flycast.png"
        }]}""")
        assertEquals(1, feed.apps.size)
        assertEquals(RepoSources.Category.EMULATORS, feed.apps.single().category)
        assertEquals(0, feed.skipped)
    }
    @Test fun categoriesPreferMetadataAndConservativelyRecognizeGamesAndEmulators() {
        val feed = RepoSources.parseFeed("""{"name":"Types","apps":[
          {"name":"Delta","bundleIdentifier":"com.rileytestut.Delta","version":"1","downloadURL":"https://example.com/a","localizedDescription":"A Nintendo console emulator"},
          {"name":"Ignited_1.8.5","bundleIdentifier":"a.ignited","version":"1","downloadURL":"https://example.com/b","localizedDescription":"Ignited v1.8.5 for iOS 15"},
          {"name":"Terraria_4.5.0","bundleIdentifier":"com.and.games505.TerrariaPaid","version":"1","downloadURL":"https://example.com/c","localizedDescription":"Terraria 4.5.0"},
          {"name":"Unknown title","bundleIdentifier":"a.game","version":"1","downloadURL":"https://example.com/d","categories":["Games"]},
          {"name":"Delta travel","bundleIdentifier":"a.travel","version":"1","downloadURL":"https://example.com/e","category":"Travel"},
          {"name":"Game news","bundleIdentifier":"a.news","version":"1","downloadURL":"https://example.com/f","description":"Read game reviews and emulator news"},
          {"name":"Other title","bundleIdentifier":"a.other","version":"1","downloadURL":"https://example.com/g","primaryGenreId":6014},
          {"name":"Sonic manual","bundleIdentifier":"a.manual","version":"1","downloadURL":"https://example.com/h","category":"Books"},
          {"name":"Unknown","bundleIdentifier":"a.unknown","version":"1","downloadURL":"https://example.com/i","description":"A puzzle game with many levels"}
        ]}""")
        assertEquals(listOf(RepoSources.Category.EMULATORS, RepoSources.Category.EMULATORS,
            RepoSources.Category.GAMES, RepoSources.Category.GAMES, RepoSources.Category.OTHER,
            RepoSources.Category.OTHER, RepoSources.Category.GAMES, RepoSources.Category.OTHER,
            RepoSources.Category.GAMES), feed.apps.map { it.category })
        assertEquals(0, feed.skipped)
    }
    private fun rejects(action: () -> Unit) {
        try { action(); fail("Expected invalid input to fail") } catch (_: Exception) {}
    }
    @Test fun sourceLinksPreserveLiteralAndEncodedPlus() {
        assertEquals("https://example.com/a+b.json?token=c+d", RepoSources.sourceURL("https://example.com/a+b.json?token=c+d"))
        assertEquals("https://example.com/a+b.json", RepoSources.sourceURL("altstore://source?url=https%3A%2F%2Fexample.com%2Fa%2Bb.json"))
        assertEquals("https://example.com/a+b.json", RepoSources.sourceURL("altstore://source?url=https://example.com/a+b.json"))
        rejects { RepoSources.sourceURL("altstore://install?url=https://example.com/a") }
        rejects { RepoSources.sourceURL("altstore://source?url=https://example.com/a&url=https://example.com/b") }
        rejects { RepoSources.sourceURL("altstore://source?url=http%3A%2F%2Fexample.com%2Fa") }
        rejects { RepoSources.sourceURL("https://user:password@example.com/a") }
    }
    @Test fun iconUrlIsOptionalAndInvalidIconDoesNotDropApp() {
        val feed = RepoSources.parseFeed("""{"name":"Icons","apps":[
          {"name":"Good","bundleIdentifier":"a.good","version":"1","downloadURL":"https://example.com/app","iconURL":"https://example.com/icon.png"},
          {"name":"Bad icon","bundleIdentifier":"a.bad","version":"1","downloadURL":"https://example.com/app","iconURL":"http://example.com/icon.png"}
        ]}""")
        assertEquals(2, feed.apps.size)
        assertEquals("https://example.com/icon.png", feed.apps[0].iconURL)
        assertNull(feed.apps[1].iconURL)
        assertEquals(0, feed.skipped)
    }
    @Test fun orderedVersionsAndLegacySourcesUseCorrectDownload() {
        val feed = RepoSources.parseFeed("""{"name":"Example","apps":[
          {"name":"Modern","bundleIdentifier":"a.modern","versions":[{"version":"2","downloadURL":"https://example.com/new"},{"version":"99","downloadURL":"https://example.com/old"}]},
          {"name":"Legacy","bundleIdentifier":"a.legacy","version":"1","downloadURL":"https://example.com/legacy"},
          {"name":"Bad","bundleIdentifier":"a.bad","versions":[{"version":"1","downloadURL":"http://example.com/bad"}]},
          {"name":"Empty","bundleIdentifier":"a.empty","versions":[]}
        ]}""")
        assertEquals(2, feed.apps.size)
        assertEquals("2", feed.apps[0].version)
        assertEquals("https://example.com/new", feed.apps[0].downloadURL)
        assertEquals("https://example.com/legacy", feed.apps[1].downloadURL)
        assertEquals(2, feed.skipped)
        rejects { RepoSources.parseFeed("not JSON") }
        rejects { RepoSources.parseFeed("""{"name":"Example","apps":{}}""") }
    }
    private fun archive(info: String, executable: String? = "Main", extra: String? = null): File {
        val file = File.createTempFile("repo-test", ".ipa")
        ZipOutputStream(file.outputStream()).use { zip ->
            zip.putNextEntry(ZipEntry("Payload/Test.app/Info.plist")); zip.write(info.toByteArray()); zip.closeEntry()
            if (executable != null) {
                zip.putNextEntry(ZipEntry("Payload/Test.app/$executable"))
                zip.write(byteArrayOf(0xcf.toByte(),0xfa.toByte(),0xed.toByte(),0xfe.toByte()) + ByteArray(28)); zip.closeEntry()
            }
            if (extra != null) { zip.putNextEntry(ZipEntry(extra)); zip.write(byteArrayOf(1)); zip.closeEntry() }
        }
        return file
    }
    @Test fun ipaRequiresActualExecutableAndRejectsTraversal() {
        val info = """<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>Main</string></dict></plist>"""
        val valid = archive(info)
        val missing = archive(info, null)
        val traversal = archive(info, extra = "../outside")
        val malformed = archive("not a plist")
        try {
            RepoSources.validateIPA(valid)
            rejects { RepoSources.validateIPA(missing) }
            rejects { RepoSources.validateIPA(traversal) }
            rejects { RepoSources.validateIPA(malformed) }
        } finally { listOf(valid, missing, traversal, malformed).forEach { it.delete() } }
    }
    @Test fun binaryPlistExecutableIsValidatedAndTruncatedTrailerFails() {
        val bytes = java.util.Base64.getDecoder().decode("YnBsaXN0MDDRAQJfEBJDRkJ1bmRsZUV4ZWN1dGFibGVUTWFpbggLIAAAAAAAAAEBAAAAAAAAAAMAAAAAAAAAAAAAAAAAAAAl")
        fun binaryArchive(metadata: ByteArray): File {
            val file = File.createTempFile("binary-repo", ".ipa")
            ZipOutputStream(file.outputStream()).use { zip ->
                zip.putNextEntry(ZipEntry("Payload/Test.app/Info.plist")); zip.write(metadata); zip.closeEntry()
                zip.putNextEntry(ZipEntry("Payload/Test.app/Main")); zip.write(byteArrayOf(0xcf.toByte(),0xfa.toByte(),0xed.toByte(),0xfe.toByte()) + ByteArray(28)); zip.closeEntry()
            }
            return file
        }
        val valid = binaryArchive(bytes)
        val broken = binaryArchive(bytes.copyOf(20))
        try { RepoSources.validateIPA(valid); rejects { RepoSources.validateIPA(broken) } }
        finally { valid.delete(); broken.delete() }
    }
}
