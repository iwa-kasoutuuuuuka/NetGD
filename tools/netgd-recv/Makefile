# netgd-recv - KallistiOS build (the Dreamcast app).  Compile-checked by CI (.github/workflows/kos.yaml);
# there is no KOS toolchain on the development PC.  Build inside the DreamSDK shell or any KOS environment:
#     make             (release)         make DEBUG=1   (-g -O0)
#     make clean
# build.ps1 runs this Makefile from PowerShell (DreamSDK).  Host tests: Makefile.host.
# Output: netgd-recv.elf  (send it with dc-tool-ip -x, e.g. through dc_auto + netgd-send, see README_ja.md)

TARGET = netgd-recv.elf

FATFS = third_party/fatfs

OBJS = src/main.o src/plat_kos.o src/dev_kos.o src/fwxfer_kos.o src/fwctl.o \
       src/crc32.o src/manifest.o src/ngblk.o src/ngfs.o src/recv_core.o src/xport_pc.o src/diskio.o \
       $(FATFS)/ff.o $(FATFS)/ffunicode.o $(FATFS)/ffsystem.o

KOS_CFLAGS += -I src -I $(FATFS) -std=gnu11 -Wall -Wno-unused-function -Wno-format-truncation \
              -DNG_JOBDIR_RAW=$(or $(NG_JOBDIR),/pc/netgd/dc1)
# NG_JOBDIR_RAW is passed WITHOUT quotes and made a string in src/xport_pc.h (as gdprobe does with
# HOST_DIR_RAW): a \"...\" inside -D does not survive kos-cc (KOS CI: missing terminating " character).
# So the path may not hold spaces, "//", quotes, or a component that is a C macro name.
ifeq ($(DEBUG),1)
  KOS_CFLAGS += -g -O0
else
  KOS_CFLAGS += -O2
endif

all: rm-elf $(TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf
	-rm -f $(OBJS)

rm-elf:
	-rm -f $(TARGET)

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS) -lm

# make run DC_IP=192.168.1.200   (normally you use dc_auto instead, see README_ja.md)
run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)
