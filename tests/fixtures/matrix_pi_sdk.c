// Deliberately no HID or vendor SDK linkage: captures the real engine's frames.
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct {
    bool connected; const char *model; uint8_t rows, columns, led_max;
    int type; bool v2; int layout; bool small, multi;
} RgbInfo;
static RgbInfo info = {true, "Wooting 80HE", 6, 17, 117, 5, true, 0, false, false};
static void record(const char *message) {
    FILE *f = fopen(getenv("MATRIX_PI_LOG"), "a");
    if (f) { fprintf(f, "%s\n", message); fclose(f); }
}
__attribute__((constructor)) static void loaded(void) { record("load"); }
bool wooting_rgb_kbd_connected(void) { record("open"); return true; }
bool wooting_rgb_close(void) { record("close"); return true; }
bool wooting_rgb_direct_set_key(uint8_t r, uint8_t c, uint8_t red, uint8_t green, uint8_t blue) { return true; }
bool wooting_rgb_array_update_keyboard(void) { return true; }
void wooting_rgb_array_auto_update(bool enabled) {}
bool wooting_rgb_array_set_full(const uint8_t *colors) {
    FILE *f = fopen(getenv("MATRIX_PI_LOG"), "a");
    if (!f) return false;
    fputs("frame ", f);
    for (unsigned int i = 0; i < 6 * 21 * 3; ++i) fprintf(f, "%02x", colors[i]);
    fputc('\n', f);
    fclose(f);
    return true;
}
RgbInfo *wooting_rgb_device_info(void) { return &info; }
int wooting_rgb_device_layout(void) { return 0; }
