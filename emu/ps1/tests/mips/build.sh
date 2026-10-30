#!/bin/sh
# build.sh name.c -> name.exe
set -e
n=${1%.c}
clang --target=mipsel-unknown-none-elf -march=mips1 -mfloat-abi=soft -O1 -ffreestanding -fno-builtin -nostdlib -G0 -fno-pic -mno-abicalls -c $1 -o $n.o 2>/dev/null
clang --target=mipsel-unknown-none-elf -march=mips1 -mfloat-abi=soft -fno-pic -mno-abicalls -c crt0.s -o crt0.o 2>/dev/null
ld.lld -T link.ld crt0.o $n.o -o $n.elf
llvm-objcopy -O binary $n.elf $n.bin
pc=$(llvm-nm $n.elf | awk '/ _start$/{print $1}')
python3 mkexe.py $n.bin $n.exe $pc
