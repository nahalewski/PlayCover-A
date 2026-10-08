/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_JPEG
#define STBI_ONLY_PNG
#define STBI_ONLY_BMP
#define STBI_ONLY_GIF
#define STBI_NO_STDIO
#include "../../../vendor/stb/stb_image.h"

// Encoding (UIImagePNGRepresentation / UIImageJPEGRepresentation).
#define STB_IMAGE_WRITE_IMPLEMENTATION
#define STBI_WRITE_NO_STDIO
#include "../../../vendor/stb/stb_image_write.h"
#include <stdlib.h>
#include <string.h>

typedef struct {
    unsigned char *data;
    size_t len;
    size_t cap;
} touchHLE_encode_buf;

static void touchHLE_encode_append(void *ctx, void *data, int size) {
    touchHLE_encode_buf *b = (touchHLE_encode_buf *)ctx;
    if (size <= 0) return;
    if (b->len + (size_t)size > b->cap) {
        size_t cap = b->cap ? b->cap : 4096;
        while (cap < b->len + (size_t)size) cap *= 2;
        unsigned char *grown = (unsigned char *)realloc(b->data, cap);
        if (!grown) return;
        b->data = grown;
        b->cap = cap;
    }
    memcpy(b->data + b->len, data, (size_t)size);
    b->len += (size_t)size;
}

// Returns a malloc()ed buffer (free with touchHLE_free_encoded) or NULL.
unsigned char *touchHLE_encode_image(int jpeg, int quality, const unsigned char *rgba,
                                     int w, int h, size_t *out_len) {
    touchHLE_encode_buf b = {0, 0, 0};
    int ok = jpeg ? stbi_write_jpg_to_func(touchHLE_encode_append, &b, w, h, 4, rgba, quality)
                  : stbi_write_png_to_func(touchHLE_encode_append, &b, w, h, 4, rgba, w * 4);
    if (!ok || b.len == 0) {
        free(b.data);
        return NULL;
    }
    *out_len = b.len;
    return b.data;
}

void touchHLE_free_encoded(void *p) { free(p); }
