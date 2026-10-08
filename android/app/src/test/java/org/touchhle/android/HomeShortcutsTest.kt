package org.touchhle.android

import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.nio.file.Files

class HomeShortcutsTest {
    @Test fun acceptsInstalledUnicodeFilenameAndAppDirectory() {
        val root = Files.createTempDirectory("shortcut-library").toFile()
        try {
            val ipa = File(root, "Pokédex Plus 2.0.ipa").apply { writeBytes(byteArrayOf(1)) }
            assertEquals(ipa.canonicalFile, HomeShortcuts.libraryFile(root, ipa.name))
            val app = File(root, "Sample.app").apply { mkdir() }
            assertEquals(app.canonicalFile, HomeShortcuts.libraryFile(root, app.name))
        } finally { root.deleteRecursively() }
    }

    @Test fun rejectsArbitraryPathsAndMissingOrNonAppFiles() {
        val root = Files.createTempDirectory("shortcut-library").toFile()
        try {
            File(root, "notes.txt").writeText("sample")
            listOf(null, "", ".", "..", "../outside.ipa", "sub/app.ipa", "sub\\app.ipa",
                "/outside.ipa", "bad\nname.ipa", "missing.ipa", "notes.txt").forEach {
                assertNull(HomeShortcuts.libraryFile(root, it))
            }
        } finally { root.deleteRecursively() }
    }

    @Test fun rejectsLibrarySymlinkPointingOutside() {
        val root = Files.createTempDirectory("shortcut-library").toFile()
        val outside = File.createTempFile("shortcut-outside", ".ipa")
        try {
            Files.createSymbolicLink(File(root, "Escape.ipa").toPath(), outside.toPath())
            assertNull(HomeShortcuts.libraryFile(root, "Escape.ipa"))
        } finally { root.deleteRecursively(); outside.delete() }
    }
}
