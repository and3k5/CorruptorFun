# mkexe.py program.bin out.exe entry_hex: wraps a flat binary loaded at
# 0x80010000 in a PS-X EXE header.
import sys, struct
binf, out, pc = sys.argv[1], sys.argv[2], int(sys.argv[3], 16)
data = open(binf, 'rb').read()
data += b'\0' * (-len(data) % 2048)
h = bytearray(2048)
h[0:8] = b'PS-X EXE'
struct.pack_into('<IIII', h, 0x10, pc, 0, 0x80010000, len(data))
struct.pack_into('<II', h, 0x30, 0x801FFF00, 0)
h[0x4C:0x4C + 22] = b'CorruptorFun test disc'
open(out, 'wb').write(h + data)
