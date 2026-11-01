// The test disc's "game": shows a picture with an Amiga-demo style sine
// scroller over it. Talks to the hardware directly, no BIOS calls.

#include "tables.h"

typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#define R32(a) (*(volatile u32 *)(a))
#define GP0 R32(0x1F801810)
#define GP1 R32(0x1F801814) // GPUSTAT when read
#define I_STAT R32(0x1F801070)
#define I_MASK R32(0x1F801074)

extern const u32 duck[]; // 320x240 15-bit pixels, from image.s

static const char MESSAGE[] = "THE BOOT FINISHED. HERE'S A DUCK.";

// 5x7 font, one byte per row, bit 4 is the leftmost column.
static const u8 FONT[][7] = {
  ['A' - 'A'] = {0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11},
  ['B' - 'A'] = {0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E},
  ['C' - 'A'] = {0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E},
  ['D' - 'A'] = {0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C},
  ['E' - 'A'] = {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F},
  ['F' - 'A'] = {0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10},
  ['G' - 'A'] = {0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F},
  ['H' - 'A'] = {0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11},
  ['I' - 'A'] = {0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E},
  ['J' - 'A'] = {0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C},
  ['K' - 'A'] = {0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11},
  ['L' - 'A'] = {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F},
  ['M' - 'A'] = {0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11},
  ['N' - 'A'] = {0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11},
  ['O' - 'A'] = {0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E},
  ['P' - 'A'] = {0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10},
  ['Q' - 'A'] = {0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D},
  ['R' - 'A'] = {0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11},
  ['S' - 'A'] = {0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E},
  ['T' - 'A'] = {0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04},
  ['U' - 'A'] = {0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E},
  ['V' - 'A'] = {0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04},
  ['W' - 'A'] = {0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A},
  ['X' - 'A'] = {0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11},
  ['Y' - 'A'] = {0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04},
  ['Z' - 'A'] = {0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F},
  [26] = {0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C}, // .
  [27] = {0x0C, 0x0C, 0x04, 0x08, 0x00, 0x00, 0x00}, // '
  [28] = {0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04}, // !
  [29] = {0}, // space
};

static const u8 *glyph(char c) {
  if (c >= 'A' && c <= 'Z') return FONT[c - 'A'];
  if (c == '.') return FONT[26];
  if (c == '\'') return FONT[27];
  if (c == '!') return FONT[28];
  return FONT[29];
}

#define SCALE 4 // font pixel size on screen
#define ADVANCE (6 * SCALE) // one character, with a column of space
#define BAND_TOP 168
#define BAND_H 68
#define TEXT_Y (BAND_TOP + BAND_H / 2 - 7 * SCALE / 2)
#define WAVE 12 // scroller wave height, pixels each way
#define HOLD_FRAMES 150

static void wait_gpu(void) {
  while (!(GP1 & (1 << 26))) {}
}

static void rect(int x, int y, int w, int h, u32 colour) {
  GP0 = 0x60000000 | colour;
  GP0 = ((u32)(y & 0xFFFF) << 16) | (x & 0xFFFF);
  GP0 = ((u32)h << 16) | w;
}

static void wait_vblank(void) {
  I_STAT = ~1u; // acknowledge the last one
  while (!(I_STAT & 1)) {}
}

static void draw(int top, int t) {
  wait_gpu();
  GP0 = 0xE1000000 | 0x400; // draw to the displayed area allowed; blending mode 0
  GP0 = 0xE3000000 | (top << 10); // drawing area: this buffer
  GP0 = 0xE4000000 | ((top + 239) << 10) | 319;
  GP0 = 0xE5000000 | (top << 11); // drawing offset

  // The picture, kept at (320, 0) in VRAM.
  GP0 = 0x80000000;
  GP0 = 320;
  GP0 = (u32)top << 16;
  GP0 = (240u << 16) | 320;

  // A darkened band behind the text (semi-transparent black).
  GP0 = 0x62000000;
  GP0 = (u32)BAND_TOP << 16;
  GP0 = ((u32)BAND_H << 16) | 320;

  // Rainbow lines along its edges, cycling.
  for (int x = 0; x < 320; x += 8) {
    u32 c = rainbow[(x / 8 + t) & 63];
    rect(x, BAND_TOP - 2, 8, 2, c);
    rect(x, BAND_TOP + BAND_H, 8, 2, rainbow[(x / 8 - t) & 63]);
  }

  // The scroller: shadows first, then the letters over them.
  int len = sizeof(MESSAGE) - 1;
  int pos = (t * 2) % (len * ADVANCE + 320);
  for (int pass = 0; pass < 2; pass++) {
    for (int i = 0; i < len; i++) {
      int cx = 320 - pos + i * ADVANCE;
      if (cx <= -ADVANCE || cx >= 320) continue;
      const u8 *g = glyph(MESSAGE[i]);
      for (int col = 0; col < 5; col++) {
        int x = cx + col * SCALE;
        int y0 = TEXT_Y + sine[(x + t * 3) & 255] * WAVE / 64;
        u32 colour = rainbow[((x >> 2) + t) & 63];
        for (int row = 0; row < 7; row++) {
          if (!(g[row] & (0x10 >> col))) continue;
          int y = y0 + row * SCALE;
          if (pass == 0) rect(x + 2, y + 2, SCALE, SCALE, 0x000000);
          else rect(x, y, SCALE, SCALE, colour);
        }
      }
    }
  }
}

int main(void) {
  __asm__ volatile("mtc0 $zero, $12"); // interrupts off: we poll instead
  I_MASK = 0;

  // Leave the licensed screen up for a couple of seconds first, like a game
  // that's still loading.
  for (int i = 0; i < HOLD_FRAMES; i++) wait_vblank();

  GP1 = 0x00000000; // reset the GPU
  GP1 = 0x06000000 | 0x260 | (0xC60 << 12); // horizontal display range
  GP1 = 0x07000000 | 16 | (256 << 10); // vertical display range
  GP1 = 0x08000001; // 320x240, NTSC, 15-bit

  // Upload the picture to (320, 0).
  wait_gpu();
  GP0 = 0x01000000;
  GP0 = 0xA0000000;
  GP0 = 320;
  GP0 = (240u << 16) | 320;
  for (int i = 0; i < 320 * 240 / 2; i++) GP0 = duck[i];

  int t = 0;
  int back = 240;
  for (;;) {
    draw(back, t);
    wait_vblank();
    GP1 = 0x05000000 | (back << 10); // show what we just drew
    if (t == 0) GP1 = 0x03000000; // display on
    back = 240 - back;
    t++;
  }
}
