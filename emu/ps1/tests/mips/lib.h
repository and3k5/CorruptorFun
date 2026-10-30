typedef unsigned int u32; typedef int s32; typedef unsigned short u16; typedef unsigned char u8;
#define R32(a) (*(volatile u32*)(a))
#define R16(a) (*(volatile u16*)(a))
#define R8(a) (*(volatile u8*)(a))
static void putc_(char c) { R8(0x1F802023) = c; }
static void puts_(const char *s) { while (*s) putc_(*s++); }
static void puthex(u32 v) { for (int i = 28; i >= 0; i -= 4) putc_("0123456789abcdef"[(v >> i) & 15]); }
static int fails;
static void check(const char *name, u32 got, u32 want) {
  if (got != want) { fails++; puts_("FAIL "); puts_(name); puts_(" got "); puthex(got); puts_(" want "); puthex(want); putc_('\n'); }
  else { puts_("ok   "); puts_(name); putc_('\n'); }
}
