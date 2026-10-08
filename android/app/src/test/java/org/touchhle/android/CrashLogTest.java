package org.touchhle.android;

import org.junit.Test;
import static org.junit.Assert.*;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

public class CrashLogTest {
    @Test public void boundedTailPreservesRecentCompleteLines() throws Exception {
        File file = File.createTempFile("playcover-log", ".txt");
        try {
            Files.write(file.toPath(), "old line\nrecent é\nPanic: real failure\n".getBytes(StandardCharsets.UTF_8));
            String tail = CrashLog.readTail(file, 30);
            assertTrue(tail.startsWith("[Earlier log omitted]\n"));
            assertTrue(tail.endsWith("Panic: real failure\n"));
            assertFalse(tail.contains("old line"));
            assertFalse(tail.contains("\ufffd"));
            assertTrue(CrashLog.readTail(file, 1000).startsWith("old line\n"));
        } finally { file.delete(); }
    }
}
