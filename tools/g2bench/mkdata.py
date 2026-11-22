#!/usr/bin/env python3
"""Creates the deterministic g2bench input file (default 32 MiB). Same pattern as g2b_gen_block().
usage: python mkdata.py <out file> [size in MiB]"""
import array, sys

def main():
    out = sys.argv[1]
    mib = int(sys.argv[2]) if len(sys.argv) > 2 else 32
    nblocks = mib * 16
    s = 0x2545F491
    base = array.array('I')
    for _ in range(16384):
        s ^= (s << 13) & 0xFFFFFFFF; s ^= s >> 17; s ^= (s << 5) & 0xFFFFFFFF
        base.append(s)
    with open(out, 'wb') as f:
        for k in range(nblocks):
            m = (k * 0x9E3779B1) & 0xFFFFFFFF
            blk = array.array('I', [x ^ m for x in base])
            if sys.byteorder == 'big': blk.byteswap()
            f.write(blk.tobytes())

if __name__ == '__main__':
    main()
