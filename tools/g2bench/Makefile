# g2bench - /pc/ fread, SH-4 CRC/FNV and thread-overlap benchmark (KallistiOS), no GD-ROM access.
# Build in the DreamSDK shell (KOS_BASE set): make [HOST_DIR_REL=/work/OpenGDEmu/tools/g2bench/host]
TARGET = g2bench.elf
OBJS = main.o g2b_core.o crc32.o
KOS_CFLAGS += -Wall -Wextra -Wno-unused-parameter
ifneq ($(HOST_DIR_REL),)
  KOS_CFLAGS += -DHOST_DIR_RAW=/pc$(HOST_DIR_REL)
endif

all: rm-elf $(TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf
	-rm -f $(OBJS)

rm-elf:
	-rm -f $(TARGET)

# the very CRC-32 / FNV-1a netgd-recv uses
crc32.o: ../netgd-recv/src/crc32.c
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS)

run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

test:
	sh tests/run_tests.sh
