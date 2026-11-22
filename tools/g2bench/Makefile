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
NET_OBJS = main_net.o g2n_core.o g2b_core.o crc32.o

all: rm-elf $(TARGET)

net: rm-net-elf $(NET_TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf rm-net-elf
	-rm -f $(OBJS) main_net.o g2n_core.o

rm-elf:
	-rm -f $(TARGET)

rm-net-elf:
	-rm -f $(NET_TARGET)

$(NET_TARGET): $(NET_OBJS)
	kos-cc -o $(NET_TARGET) $(NET_OBJS)

# the very CRC-32 / FNV-1a netgd-recv uses
crc32.o: ../netgd-recv/src/crc32.c
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS)

run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

test:
	sh tests/run_tests.sh
