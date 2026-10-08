package org.touchhle.android;

import java.io.File;
import java.io.IOException;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;

/** Reads only the bounded tail of the runtime's existing log file. */
final class CrashLog {
    static String readTail(File file, int limit) throws IOException {
        if (limit <= 0 || limit > 1024 * 1024) throw new IllegalArgumentException("Invalid log limit");
        try (RandomAccessFile input = new RandomAccessFile(file, "r")) {
            long length = input.length();
            long start = Math.max(0, length - limit);
            input.seek(start);
            byte[] bytes = new byte[(int) (length - start)];
            input.readFully(bytes);
            int offset = 0;
            // Do not show a partial line or a split UTF-8 code point.
            if (start > 0) {
                while (offset < bytes.length && bytes[offset] != '\n') offset++;
                if (offset < bytes.length) offset++;
                else offset = 0;
                while (offset < bytes.length && (bytes[offset] & 0xc0) == 0x80) offset++;
            }
            return (start > 0 ? "[Earlier log omitted]\n" : "") +
                new String(bytes, offset, bytes.length - offset, StandardCharsets.UTF_8);
        }
    }
}
