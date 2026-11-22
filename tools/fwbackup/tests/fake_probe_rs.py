#!/usr/bin/env python3
"""Fake probe-rs for fwbackup tests.  Emulates `list`, `info`, `read`, `--version`.

Environment:
  FAKE_SCENARIO  unlocked2e | unlocked4e | locked | notarget | noprobe | multiprobe | badchip | noresp
  FAKE_STATE     directory for counters / invocation log (required)
  FAKE_FMT       normal | crlf | upper | noaddr | noise | simple (output-format edge cases)
  FAKE_FLAKY     hex address: the first read covering it returns one flipped byte
  FAKE_ALWAYS_FLAKY  hex address: every read covering it differs (unstable)
  FAKE_VERSION   version number printed by --version (default 0.32.0)
  FAKE_CIDR      hex CHIPID_CIDR override (default per scenario)
  FAKE_SCENARIO also: oldrs (rejects options) | inaccessible (probe listed as inaccessible)
  FAKE_FLASH     optional path to a flash image (128 KB); default = built-in pattern
Output formats follow probe-rs read.rs (write_hex_table / write_simple_hex).
"""
import os
import sys

FLASH_BASE, FLASH_SIZE = 0x80000, 0x20000


def default_flash():
    buf = bytearray(FLASH_SIZE)
    for i in range(FLASH_SIZE):
        buf[i] = (i * 131 + (i >> 8) * 7 + 13) & 0xFF
    buf[0:4] = (0x20004000).to_bytes(4, "little")
    buf[4:8] = (0x00080201).to_bytes(4, "little")
    return bytes(buf)


def main():
    argv = sys.argv[1:]
    state = os.environ["FAKE_STATE"]
    scen = os.environ.get("FAKE_SCENARIO", "unlocked2e")
    fmt = os.environ.get("FAKE_FMT", "normal")
    with open(os.path.join(state, "calls.log"), "a", encoding="utf-8") as f:
        f.write(" ".join(argv) + "\n")
    if argv == ["--version"]:
        print("probe-rs %s (git commit: fake)" % os.environ.get("FAKE_VERSION", "0.32.0"))
        return 0
    sub = argv[0]
    if sub == "list":
        if scen == "noprobe":
            print("No debug probes were found.")
            return 0
        print("The following debug probes were found:")
        if scen == "inaccessible":
            print("[0]: Debugprobe on Pico (CMSIS-DAP) -- 2e8a:000c:E6614103E7123456 (CMSIS-DAP) (inaccessible)")
            return 0
        print("[0]: Debugprobe on Pico (CMSIS-DAP) -- 2e8a:000c:E6614103E7123456 (CMSIS-DAP)")
        if scen == "multiprobe":
            print("[1]: Other probe -- 1234:5678:ABC (CMSIS-DAP)")
        return 0
    if sub == "info" and scen == "oldrs":
        print("error: unexpected argument '--verbose' found", file=sys.stderr)
        return 2
    if sub == "info":
        if scen == "locked":
            print("Probing target via SWD\n----------------------\n")
            print("Debug Port: DPv1, Designer: ARM Ltd\n`-- 0 MemoryAP\n    `-- Error: access denied")
            return 0
        print("Probing target via SWD\n----------------------\n")
        print("Error showing ARM chip information: Failed to connect: no ACK", file=sys.stderr)
        print("Note: `info` only tries to identify the debug port and its components.")
        return 1 if scen != "multiprobe" else 0
    if sub != "read":
        print("fake: unsupported " + sub, file=sys.stderr)
        return 64
    # parse read args
    pos, i = [], 1
    chip = None
    while i < len(argv):
        a = argv[i]
        if a in ("--probe", "--protocol", "--chip", "--speed"):
            if a == "--chip":
                chip = argv[i + 1]
            i += 2
            continue
        if a.startswith("--"):
            i += 1
            continue
        pos.append(a)
        i += 1
    width, addr, words = pos[0], int(pos[1], 0), int(pos[2])
    assert width == "b32"
    if scen == "oldrs":
        print("error: unexpected argument '--non-interactive' found", file=sys.stderr)
        return 2
    if scen in ("noprobe", "multiprobe", "notarget"):
        print("Error: Failed to open the debug probe / attach: no response from target", file=sys.stderr)
        return 1
    if scen == "noresp":
        print("Error: Connecting to the chip was unsuccessful.\n"
              "Caused by:\n"
              "    0: An ARM specific error occurred.\n"
              "    1: No target responded to the debug port. Check that the target is connected and powered, "
              "and that the SWD/JTAG wiring is correct.\n"
              "    2: An error occurred in the communication with an access port or debug port.\n"
              "    3: Target device did not respond to request.", file=sys.stderr)
        return 1
    if scen == "badchip":
        print("Error: The connection to the chip could not be established: The requested chip 'ATSAM3U9Z' was not found in the list of known targets.", file=sys.stderr)
        return 1
    if scen == "locked":
        print("Error: Failed to read memory: AP access denied (FAULT)", file=sys.stderr)
        return 1
    n = words * 4
    if addr == 0x400E0740:
        cidr = 0x281A0761 if scen == "unlocked2e" else 0x28100961
        if os.environ.get("FAKE_CIDR"):
            cidr = int(os.environ["FAKE_CIDR"], 16)
        data = cidr.to_bytes(4, "little") + (0).to_bytes(4, "little")
        data = data[:n]
    elif FLASH_BASE <= addr and addr + n <= FLASH_BASE + FLASH_SIZE:
        path = os.environ.get("FAKE_FLASH")
        flash = open(path, "rb").read() if path else default_flash()
        data = bytearray(flash[addr - FLASH_BASE: addr - FLASH_BASE + n])
        flaky = os.environ.get("FAKE_FLAKY")
        always = os.environ.get("FAKE_ALWAYS_FLAKY")
        if flaky and addr <= int(flaky, 16) < addr + n:
            marker = os.path.join(state, "flaky.done")
            if not os.path.exists(marker):
                open(marker, "w").close()
                data[int(flaky, 16) - addr] ^= 0x55
        if always and addr <= int(always, 16) < addr + n:
            cnt = os.path.join(state, "always.cnt")
            c = int(open(cnt).read()) if os.path.exists(cnt) else 0
            open(cnt, "w").write(str(c + 1))
            data[int(always, 16) - addr] ^= (c % 250) + 1
        data = bytes(data)
    else:
        print("Error: memory access failed at unmapped address", file=sys.stderr)
        return 1
    eol = "\r\n" if fmt == "crlf" else "\n"
    out = []
    if fmt == "noise":
        out.append("WARN probe_rs::probe: some warning 12345678 abcdef01")
    if fmt == "simple":
        out.append(" ".join(data[j:j + 4][::-1].hex() for j in range(0, n, 4)))
    else:
        for off in range(0, n, 32):
            line = data[off:off + 32]
            words_s = " ".join(line[j:j + 4][::-1].hex() for j in range(0, len(line), 4))
            if fmt == "upper":
                words_s = words_s.upper()
            out.append(words_s if fmt == "noaddr" else f"{addr + off:08x}: {words_s}")
    if fmt == "noise":
        out.append("")
    sys.stdout.write(eol.join(out) + eol)
    return 0


if __name__ == "__main__":
    sys.exit(main())
