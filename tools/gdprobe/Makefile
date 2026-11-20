# gdprobe - read-only GD-ROM drive probe (KallistiOS 2.3.0).
# Build in the DreamSDK shell (KOS_BASE must be set, i.e. environ.sh sourced):
#   make                      release build -> gdprobe.elf
#   make HOST_DIR_REL=/work/OpenGDEmu/tools/gdprobe/host
#                             /pc/ folder the program uses (as seen by dc-tool-ip), see build.ps1
# Mirrors sample/hello_bba/Makefile of the Dreamcast-Dev-Tools-for-BBA repo.

TARGET = gdprobe.elf

OBJS = main.o gdraw.o probes.o record.o pcout.o hash.o cfg.o whitelist.o

KOS_CFLAGS += -Wall -Wextra -Wno-unused-parameter

# Folder on the PC through dc-tool-ip's /pc/. Passed unquoted and stringified in
# main.c, so the path must not contain spaces.
ifneq ($(HOST_DIR_REL),)
  KOS_CFLAGS += -DHOST_DIR_RAW=/pc$(HOST_DIR_REL)
endif

all: rm-elf $(TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf
	-rm -f $(OBJS)

rm-elf:
	-rm -f $(TARGET)

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS)

# make run DC_IP=192.168.10.2
run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

# Host tests (plain gcc, no KOS): see tests/run_tests.sh
test:
	sh tests/run_tests.sh
