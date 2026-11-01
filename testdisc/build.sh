#!/bin/sh
# Builds the test disc's program into ../src/assets/testdisc.exe.
# Needs clang, ld.lld, llvm-objcopy and Python with Pillow.
set -e
cd "$(dirname "$0")"
python3 mkdata.py
CC="clang --target=mipsel-unknown-none-elf -march=mips1 -mfloat-abi=soft -fno-pic -mno-abicalls"
$CC -O2 -ffreestanding -fno-builtin -nostdlib -G0 -c duck.c -o duck.o
$CC -c crt0.s -o crt0.o
$CC -c image.s -o image.o
ld.lld -T link.ld crt0.o duck.o image.o -o duck.elf
llvm-objcopy -O binary duck.elf duck.bin
pc=$(llvm-nm duck.elf | awk '/ _start$/{print $1}')
python3 mkexe.py duck.bin ../src/assets/testdisc.exe "$pc"
rm -f duck.o crt0.o image.o duck.elf duck.bin duck.raw
ls -l ../src/assets/testdisc.exe
