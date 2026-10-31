#include "lib.h"
#define GP0(v) R32(0x1F801810) = (v)
#define GP1(v) R32(0x1F801814) = (v)
static void wait_ready(void) { while (!(R32(0x1F801814) & (1 << 26))); }
static u32 xy(int x, int y) { return ((u32)(y & 0xFFFF) << 16) | (x & 0xFFFF); }
int main(void) {
  GP1(0); GP1(0x03000000); GP1(0x08000001); GP1(0x06C60260); GP1(0x07042018); GP1(0x05000000);
  GP0(0xE1000200 | 0x400); GP0(0xE3000000); GP0(0xE4000000 | (239 << 10) | 319); GP0(0xE5000000);
  /* background fill */
  GP0(0x02302010); GP0(0); GP0(xy(320, 240));
  /* flat triangle */
  GP0(0x200000FF); GP0(xy(20, 20)); GP0(xy(120, 30)); GP0(xy(40, 110));
  /* gouraud quad */
  GP0(0x380000FF); GP0(xy(140, 20)); GP0(0x0000FF00); GP0(xy(300, 20)); GP0(0x00FF0000); GP0(xy(140, 110)); GP0(0x00FFFFFF); GP0(xy(300, 110));
  /* upload a 4bpp 16x16 checker texture at (640,0) and a CLUT at (0,480) */
  GP0(0xA0000000); GP0(xy(640, 0)); GP0(xy(4, 16));
  for (int y = 0; y < 16; y++) for (int i = 0; i < 2; i++) { u32 w = 0; for (int k = 0; k < 8; k++) { int x = i * 8 + k; w |= (u32)(((x / 4 + y / 4) & 1) ? 1 : 2) << (k * 4); } GP0(w); }
  GP0(0xA0000000); GP0(xy(0, 480)); GP0(xy(16, 1));
  GP0(0x00000000 | (0x7C00u << 16)); /* idx0 = 0 (transparent), idx1 = blue */
  GP0(0x0000001F); /* idx2 = red, idx3 = 0 */
  for (int i = 0; i < 6; i++) GP0(0);
  /* textured quad (raw), texpage x=640 -> 10, 4bpp; clut at (0,480) -> 480<<6 */
  u32 clut = (480 << 6) | 0; u32 tpage = 10 | (0 << 7);
  GP0(0x2D808080); GP0(xy(20, 130)); GP0((clut << 16) | 0x0000); GP0(xy(100, 130)); GP0((tpage << 16) | 0x000F); GP0(xy(20, 210)); GP0(0x0F00); GP0(xy(100, 210)); GP0(0x0F0F);
  /* semi transparent rect (mode 0: 50/50) */
  GP0(0xE1000000 | 0x200 | (0 << 5));
  GP0(0x6200FF00); GP0(xy(60, 60)); GP0(xy(120, 100));
  /* shaded line and a polyline */
  GP0(0x500000FF); GP0(xy(140, 130)); GP0(0x00FFFF00); GP0(xy(300, 220));
  GP0(0x48FFFFFF); GP0(xy(140, 220)); GP0(xy(200, 140)); GP0(xy(260, 220)); GP0(0x55555555);
  /* 16x16 sprite of the texture */
  GP0(0xE100000A); GP0(0x7D808080); GP0(xy(120, 150)); GP0((clut << 16) | 0);
  puts_("drawn\n");
  wait_ready();
  for (;;);
}
