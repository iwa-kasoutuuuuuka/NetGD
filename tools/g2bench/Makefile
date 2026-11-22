# g2bench - /pc/ fread, SH-4 CRC/FNV and thread-overlap benchmark (KallistiOS), no GD-ROM access.
# Build in the DreamSDK shell (KOS_BASE set): make [HOST_DIR_REL=/work/OpenGDEmu/tools/g2bench/host]
TARGET = g2bench.elf
OBJS = main.o g2b_core.o crc32.o
KOS_CFLAGS += -Wall -Wextra -Wno-unused-parameter
ifneq ($(HOST_DIR_REL),)
  KOS_CFLAGS += -DHOST_DIR_RAW=/pc$(HOST_DIR_REL)
endif

ifneq ($(G2B_MAX_BS),)
  KOS_CFLAGS += -DG2B_MAX_BS=$(G2B_MAX_BS)
endif

# phase D: the same checksums, but the data comes over KOS's TCP/IP stack (INIT_NET): make g2netbench.elf
NET_TARGET = g2netbench.elf
NET_OBJS = main_net.o g2n_core.o g2x_core.o g2b_core.o crc32.o
ifneq ($(G2N_LISTEN_RCVBUF),)
  KOS_CFLAGS += -DG2N_LISTEN_RCVBUF=$(G2N_LISTEN_RCVBUF)
endif
ifneq ($(G2N_TCP_ALL),)
  KOS_CFLAGS += -DG2N_TCP_ALL=$(G2N_TCP_ALL)
endif

# the same benchmark linked with a PATCHED COPY of KOS's kernel/net/net_tcp.c (tools/kos-patches/
# net_tcp-rx-fix.patch). The copy defines net_tcp_init / net_tcp_shutdown, the only global symbols of KOS's
# net_tcp.o, so the linker never pulls that member out of libkallisti.a (objects come before -lkallisti).
# DreamSDK itself is not modified: the source is copied and patched under build-fix/.
FIX_TARGET = g2netbench_tcpfix.elf
FIX_OBJS = main_net_fix.o net_tcp_fix.o g2n_core.o g2x_core.o g2b_core.o crc32.o
KOS_NET_SRC = $(KOS_BASE)/kernel/net
FIX_PATCH = ../kos-patches/net_tcp-rx-fix.patch

all: rm-elf $(TARGET)

net: rm-net-elf $(NET_TARGET)

net-tcpfix: rm-fix-elf $(FIX_TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf rm-net-elf rm-fix-elf
	-rm -f $(OBJS) main_net.o g2n_core.o g2x_core.o main_net_fix.o net_tcp_fix.o
	-rm -rf build-fix

rm-elf:
	-rm -f $(TARGET)

rm-net-elf:
	-rm -f $(NET_TARGET)

rm-fix-elf:
	-rm -f $(FIX_TARGET)

$(NET_TARGET): $(NET_OBJS)
	kos-cc -o $(NET_TARGET) $(NET_OBJS)

build-fix/net_tcp_fix.c: $(KOS_NET_SRC)/net_tcp.c $(FIX_PATCH)
	mkdir -p build-fix
	tr -d '\r' < $(KOS_NET_SRC)/net_tcp.c > build-fix/net_tcp.orig.c
	tr -d '\r' < $(FIX_PATCH) > build-fix/fix.patch
	patch -s -o $@ build-fix/net_tcp.orig.c build-fix/fix.patch

net_tcp_fix.o: build-fix/net_tcp_fix.c
	kos-cc $(KOS_CFLAGS) -I$(KOS_NET_SRC) -c $< -o $@

main_net_fix.o: main_net.c
	kos-cc $(KOS_CFLAGS) -DG2N_TCPFIX=1 -c $< -o $@

# -Map: the check below reads which object net_tcp_init came from
$(FIX_TARGET): $(FIX_OBJS)
	kos-cc -o $(FIX_TARGET) $(FIX_OBJS) -Wl,-Map,build-fix/tcpfix.map
	@grep -q 'net_tcp_fix_wnd_updates' build-fix/tcpfix.map || { echo 'patched net_tcp.c not linked'; exit 1; }
	@if grep -q 'libkallisti.a(net_tcp.o)' build-fix/tcpfix.map; then echo "KOS's own net_tcp.o was linked too"; exit 1; fi
	@echo 'tcpfix: patched net_tcp.c linked, libkallisti.a(net_tcp.o) not pulled'

# the very CRC-32 / FNV-1a netgd-recv uses
crc32.o: ../netgd-recv/src/crc32.c
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS)

run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

test:
	sh tests/run_tests.sh
