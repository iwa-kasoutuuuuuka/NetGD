# netgd-fwupd - update NetGD's firmware over the secondary IDE device (KallistiOS 2.3.0).
# Build in the DreamSDK shell (KOS_BASE must be set, i.e. environ.sh sourced):
#   make                      release build -> netgd-fwupd.elf
#   make HOST_DIR_REL=/work/OpenGDEmu/tools/netgd-fwupd/host
#                             /pc/ folder the program uses (as seen by dc-tool-ip), see build.ps1
#   make NG_FWIMAGE=/pc/work/x/netgd.ogfw
#                             update image path compiled in (default: <HOST_DIR>/netgd.ogfw;
#                             at run time <HOST_DIR>/netgd-fwupd.cfg can say  image=<path>)
# Same layout as tools/gdprobe/Makefile; .github/workflows/kos.yaml builds every tools/*/Makefile
# that includes Makefile.rules, so keep that line.

TARGET = netgd-fwupd.elf

OBJS = main.o g1dev.o fwcore.o fwmsg.o resout.o

KOS_CFLAGS += -Wall -Wextra -Wno-unused-parameter

# Folder on the PC through dc-tool-ip's /pc/. Passed unquoted and stringified in
# main.c, so the path must not contain spaces.
ifneq ($(HOST_DIR_REL),)
  KOS_CFLAGS += -DHOST_DIR_RAW=/pc$(HOST_DIR_REL)
endif
ifneq ($(NG_FWIMAGE),)
  KOS_CFLAGS += -DFW_IMAGE_RAW=$(NG_FWIMAGE)
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

# fwmsg_gen.h is generated from fwmsg.def and committed; this regenerates it.
msgs:
	python gen_msgs.py
