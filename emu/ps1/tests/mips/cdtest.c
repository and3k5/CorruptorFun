#include "lib.h"
#define CD(i) R8(0x1F801800 + (i))
static u8 resp[16]; static int rlen;
static int wait_int(void) {
  for (int t = 0; t < 2000000; t++) {
    CD(0) = 1; int f = CD(3) & 7;
    if (f) { rlen = 0; while ((CD(0) & 0x20) && rlen < 16) resp[rlen++] = CD(1); CD(0) = 1; CD(3) = 0x1F; return f; }
  }
  return -1;
}
static void cmd(u8 c, const u8 *p, int n) { CD(0) = 0; for (int i = 0; i < n; i++) CD(2) = p[i]; CD(0) = 0; CD(1) = c; }
int main(void) {
  CD(0) = 1; CD(2) = 0x1F; /* enable all INTs */
  cmd(0x0A, 0, 0); check("init int3", wait_int(), 3); check("init int2", wait_int(), 2);
  cmd(0x01, 0, 0); check("getstat int3", wait_int(), 3);
  u8 p20 = 0x20; cmd(0x19, &p20, 1); check("test 20", wait_int(), 3); check("cd bios year", resp[0], 0x94);
  cmd(0x1A, 0, 0);
  int f = wait_int();
  if (f == 5) { check("getid no disc: lid open", resp[0] & 0x10, 0x10); check("getid no disc: not ready", resp[1], 0x80); puts_("no disc path\n"); }
  else {
    check("getid int3", f, 3); f = wait_int();
    check("getid licensed int2", f, 2); check("getid S", resp[4], 'S'); 
    u8 mode = 0x80; cmd(0x0E, &mode, 1); check("setmode", wait_int(), 3);
    u8 loc[3] = {0x00, 0x02, 0x16}; cmd(0x02, loc, 3); check("setloc", wait_int(), 3);
    cmd(0x06, 0, 0); check("readn int3", wait_int(), 3); check("readn int1", wait_int(), 1);
    CD(0) = 0; CD(3) = 0x80;
    static u32 buf[512];
    R32(0x1F8010F0) |= 0x8000; R32(0x1F8010B0) = (u32)buf; R32(0x1F8010B4) = 0x10200; R32(0x1F8010B8) = 0x11000000;
    check("sector 16 is PVD", (buf[0] >> 8) & 0xFFFFFF, 'C' | ('D' << 8) | ('0' << 16));
    cmd(0x09, 0, 0); wait_int(); 
  }
  puts_(fails ? "SOME TESTS FAILED\n" : "ALL PASSED\n");
  for (;;);
}
