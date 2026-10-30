#include "lib.h"
static volatile u32 mem[4];
volatile u32 exc_count, exc_cause, irq_count;

u32 load_delay_old(void) {
  u32 r; mem[0] = 5;
  __asm__ volatile(".set noreorder\n li $t1, 1\n lw $t1, 0(%1)\n move %0, $t1\n nop\n .set reorder" : "=r"(r) : "r"(mem) : "t1");
  return r; /* old value 1 */
}
u32 load_delay_new(void) {
  u32 r; mem[0] = 5;
  __asm__ volatile(".set noreorder\n li $t1, 1\n lw $t1, 0(%1)\n nop\n move %0, $t1\n .set reorder" : "=r"(r) : "r"(mem) : "t1");
  return r; /* 5 */
}
u32 load_then_write(void) {
  u32 r; mem[0] = 5;
  __asm__ volatile(".set noreorder\n lw $t1, 0(%1)\n li $t1, 7\n nop\n move %0, $t1\n .set reorder" : "=r"(r) : "r"(mem) : "t1");
  return r; /* 7: the instruction write wins */
}
u32 mfc0_delay(void) {
  u32 r;
  __asm__ volatile(".set noreorder\n li $t1, 0x55\n mfc0 $t1, $15\n move %0, $t1\n nop\n .set reorder" : "=r"(r) :: "t1");
  return r; /* old 0x55 */
}
u32 lwl_lwr(u32 off) {
  u32 r; mem[0] = 0x44332211; mem[1] = 0x88776655;
  u8 *p = (u8*)mem + off;
  __asm__ volatile(".set noreorder\n li %0, 0\n lwl %0, 3(%1)\n lwr %0, 0(%1)\n nop\n .set reorder" : "=&r"(r) : "r"(p));
  return r;
}
u32 lwr_lwl_chain(void) {
  /* lwr then lwl on the same register back to back must merge the in-flight value */
  u32 r; mem[0] = 0x44332211; mem[1] = 0x88776655;
  u8 *p = (u8*)mem + 1;
  __asm__ volatile(".set noreorder\n li %0, 0xAAAAAAAA\n lwr %0, 0(%1)\n lwl %0, 3(%1)\n nop\n .set reorder" : "=&r"(r) : "r"(p));
  return r; /* 0x55443322 */
}
u32 swl_swr(void) {
  mem[0] = 0; mem[1] = 0;
  u8 *p = (u8*)mem + 1; u32 v = 0xAABBCCDD;
  __asm__ volatile("swl %0, 3(%1)\n swr %0, 0(%1)" :: "r"(v), "r"(p) : "memory");
  return mem[0] ^ mem[1];
}
u32 bltzal_link(void) {
  u32 ra_val;
  __asm__ volatile(".set noreorder\n move $t2, $ra\n li $t0, 1\n bltzal $t0, 1f\n nop\n move %0, $ra\n move $ra, $t2\n 1:\n .set reorder" : "=r"(ra_val) :: "t0", "t2", "ra");
  return ra_val != 0;
}
s32 sdiv(s32 a, s32 b) { s32 q; __asm__ volatile("div $zero, %1, %2\n mflo %0" : "=r"(q) : "r"(a), "r"(b)); return q; }
s32 srem(s32 a, s32 b) { s32 q; __asm__ volatile("div $zero, %1, %2\n mfhi %0" : "=r"(q) : "r"(a), "r"(b)); return q; }
u32 udiv(u32 a, u32 b) { u32 q; __asm__ volatile("divu $zero, %1, %2\n mflo %0" : "=r"(q) : "r"(a), "r"(b)); return q; }

extern char handler_start[], handler_end[];
__asm__(".set noreorder\n.global handler_start\nhandler_start:\n"
        " lui $k0, %hi(exc_entry)\n addiu $k0, $k0, %lo(exc_entry)\n jr $k0\n nop\n"
        ".global handler_end\nhandler_end:\n"
        "exc_entry:\n"
        " lui $k1, %hi(exc_count)\n lw $k0, %lo(exc_count)($k1)\n nop\n addiu $k0, $k0, 1\n sw $k0, %lo(exc_count)($k1)\n"
        " mfc0 $k0, $13\n nop\n lui $k1, %hi(exc_cause)\n sw $k0, %lo(exc_cause)($k1)\n"
        " andi $k0, $k0, 0x7c\n bnez $k0, 1f\n nop\n"
        /* interrupt: count and ack I_STAT */
        " lui $k1, %hi(irq_count)\n lw $k0, %lo(irq_count)($k1)\n nop\n addiu $k0, $k0, 1\n sw $k0, %lo(irq_count)($k1)\n"
        " lui $k1, 0x1f80\n sw $zero, 0x1070($k1)\n"
        " mfc0 $k0, $14\n nop\n jr $k0\n .word 0x42000010\n"
        "1: mfc0 $k0, $14\n nop\n addiu $k0, $k0, 4\n jr $k0\n .word 0x42000010\n.set reorder\n");

static void install_handler(void) {
  u32 *dst = (u32*)0x80000080; u32 *src = (u32*)handler_start;
  for (int i = 0; i < (handler_end - handler_start) / 4; i++) dst[i] = src[i];
}

int main(void) {
  puts_("cpu tests\n");
  check("load delay: slot sees old value", load_delay_old(), 1);
  check("load delay: next sees new value", load_delay_new(), 5);
  check("load delay: write in slot wins", load_then_write(), 7);
  check("mfc0 has a delay slot", mfc0_delay(), 0x55);
  check("lwl/lwr +0", lwl_lwr(0), 0x44332211);
  check("lwl/lwr +1", lwl_lwr(1), 0x55443322);
  check("lwl/lwr +2", lwl_lwr(2), 0x66554433);
  check("lwl/lwr +3", lwl_lwr(3), 0x77665544);
  check("lwr then lwl chained", lwr_lwl_chain(), 0x55443322);
  check("swl/swr +1", swl_swr(), 0xAABBCCDD << 8 ^ 0xAA);
  check("bltzal links when not taken", bltzal_link(), 1);
  check("div", sdiv(-7, 2), (u32)-3);
  check("rem", srem(-7, 2), (u32)-1);
  check("div by 0 (pos)", sdiv(5, 0), 0xFFFFFFFF);
  check("div by 0 (neg)", sdiv(-5, 0), 1);
  check("rem by 0", srem(-5, 0), (u32)-5);
  check("div overflow", sdiv(0x80000000, -1), 0x80000000);
  check("divu by 0", udiv(9, 0), 0xFFFFFFFF);
  volatile s32 a = -16; check("sra", (u32)(a >> 2), (u32)-4);
  volatile u32 b = 0x80000000; check("srl", b >> 31, 1);

  install_handler();
  exc_count = 0;
  __asm__ volatile("syscall");
  check("syscall handled", exc_count, 1);
  check("syscall cause", (exc_cause >> 2) & 31, 8);
  exc_count = 0;
  __asm__ volatile(".set noreorder\n li $t0, 0x7fffffff\n add $t1, $t0, $t0\n .set reorder" ::: "t0", "t1");
  check("overflow exception", (exc_cause >> 2) & 31, 12);

  /* vblank interrupt */
  irq_count = 0;
  R32(0x1F801070) = 0;
  R32(0x1F801074) = 1; /* vblank */
  u32 sr; __asm__ volatile("mfc0 %0, $12\n nop" : "=r"(sr));
  sr |= 0x401; sr &= ~(1 << 22);
  __asm__ volatile("mtc0 %0, $12" :: "r"(sr));
  for (volatile int i = 0; i < 2000000 && irq_count < 3; i++);
  check("vblank irqs arrive", irq_count >= 3, 1);
  sr &= ~1; __asm__ volatile("mtc0 %0, $12" :: "r"(sr));

  /* DMA6 OTC */
  static u32 ot[8];
  R32(0x1F8010F0) |= 0x08000000;
  R32(0x1F8010E0) = (u32)&ot[7];
  R32(0x1F8010E4) = 8;
  R32(0x1F8010E8) = 0x11000002;
  check("otc first", ot[7], ((u32)&ot[6]) & 0x1FFFFF);
  check("otc last", ot[0], 0x00FFFFFF);

  /* timer 2 counts sysclk/8 */
  R32(0x1F801124) = 0x200; u32 t0 = R32(0x1F801120);
  for (volatile int i = 0; i < 1000; i++);
  check("timer2 runs", R32(0x1F801120) > t0, 1);

  /* GTE RTPS */
  u32 sr2; __asm__ volatile("mfc0 %0, $12\n nop" : "=r"(sr2));
  sr2 |= 0x40000000; __asm__ volatile("mtc0 %0, $12" :: "r"(sr2));
  __asm__ volatile(
    "li $t0, 0x1000\n .word 0x48C80000\n .word 0x48C00800\n .word 0x48C81000\n .word 0x48C01800\n .word 0x48C82000\n"
    ".word 0x48C02800\n .word 0x48C03000\n .word 0x48C03800\n"
    "li $t0, 0xA00000\n .word 0x48C8C000\n li $t0, 0x780000\n .word 0x48C8C800\n li $t0, 200\n .word 0x48C8D000\n"
    "li $t0, 0x000A0014\n .word 0x48880000\n li $t0, 400\n .word 0x48880800\n nop\n nop\n"
    ".word 0x4A180001\n" ::: "t0");
  u32 sxy; __asm__ volatile(".word 0x48087000\n nop\n move %0, $t0" : "=r"(sxy) :: "t0");
  check("gte rtps sxy", sxy, (125u << 16) | 170);

  puts_(fails ? "SOME TESTS FAILED\n" : "ALL PASSED\n");
  return 0;
}
