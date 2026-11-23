# netgd-t2 - the standalone T2 receiver (NGTP over KOS TCP -> NetGD SD card), KallistiOS build.  NOT run on hardware.
# Build in the DreamSDK shell (KOS_BASE set), or with build.ps1 from PowerShell:
#     make                 netgd-t2.elf          stock KOS TCP: WANT 16 KiB x 1
#     make tcpfix          netgd-t2-tcpfix.elf   KOS's net_tcp.c replaced by a patched copy (../kos-patches): 16 KiB x 1 for now
#                          (64 KiB x 2 is the shape that hung the real Dreamcast in g2netbench p4, cause unknown;
#                          make tcpfix T2_WANT=65536 T2_DEPTH=2 asks for it)
#     make T2_NO_DRIVE=1   never sends a packet to the drive (network-only smoke test; every write is refused)
#     make T2_WANT=32768 T2_DEPTH=2 T2_RCVBUF=20480 T2_CODE=1234 T2_IP=0xC0A80A02   (numbers only: no quoted -D strings)
#     make clean
# Objects go to build/kos (build/kos-fix for tcpfix), never next to the sources (netgd-menu, netgd-recv are shared:
#   src/X.c -> X.o, ../netgd-menu/src/X.c -> menu_X.o, ../netgd-recv/src/X.c -> recv_X.o, FatFs -> ff_X.o).
# DreamSDK is not modified: for tcpfix, $(KOS_BASE)/kernel/net/net_tcp.c is copied, patched and compiled here, and the
# link map is checked (the patched copy is in, libkallisti.a(net_tcp.o) is not), as tools/g2bench (feat/g2-transport).
# The patch file is used as it is (swappable: a fixed version only replaces ../kos-patches/net_tcp-rx-fix.patch).

MENU   = ../netgd-menu
RECV   = ../netgd-recv
FATFS  = $(RECV)/third_party/fatfs
PATCH  = ../kos-patches/net_tcp-rx-fix.patch
KOS_NET_SRC = $(KOS_BASE)/kernel/net

ifeq ($(TCPFIX),1)
  TARGET = netgd-t2-tcpfix.elf
  OBJDIR = build/kos-fix
  KOS_CFLAGS += -DT2_TCPFIX=1
  FIX_OBJS = $(OBJDIR)/net_tcp_fix.o
  MAP = -Wl,-Map,$(OBJDIR)/t2.map
else
  TARGET = netgd-t2.elf
  OBJDIR = build/kos
  FIX_OBJS =
  MAP =
endif

T2_OBJS   = $(addprefix $(OBJDIR)/, t2_app.o main_kos.o)
MENU_OBJS = $(addprefix $(OBJDIR)/menu_, nm_screen.o ngtp_frame.o ngtp_srv.o plat_kos.o net_kos.o nm_netup.o)
RECV_OBJS = $(addprefix $(OBJDIR)/recv_, crc32.o manifest.o diskio.o ngblk.o ngfs.o fwctl.o recv_core.o dev_kos.o fwxfer_kos.o)
FATFS_OBJS = $(addprefix $(OBJDIR)/ff_, ff.o ffunicode.o ffsystem.o)
OBJS = $(T2_OBJS) $(MENU_OBJS) $(RECV_OBJS) $(FATFS_OBJS) $(FIX_OBJS)

KOS_CFLAGS += -I src -I $(MENU)/src -I $(RECV)/src -I $(FATFS) -std=gnu11 -Wall -Wno-unused-function -Wno-format-truncation
ifneq ($(T2_WANT),)
  KOS_CFLAGS += -DT2_WANT=$(T2_WANT)
endif
ifneq ($(T2_DEPTH),)
  KOS_CFLAGS += -DT2_DEPTH=$(T2_DEPTH)
endif
ifneq ($(T2_RCVBUF),)
  KOS_CFLAGS += -DT2_RCVBUF=$(T2_RCVBUF)
endif
ifneq ($(T2_CODE),)
  KOS_CFLAGS += -DT2_CODE=$(T2_CODE)
endif
ifneq ($(T2_IP),)
  KOS_CFLAGS += -DT2_IP=$(T2_IP)
endif
ifneq ($(T2_NO_DRIVE),)
  KOS_CFLAGS += -DT2_NO_DRIVE=$(T2_NO_DRIVE)
endif
ifeq ($(DEBUG),1)
  KOS_CFLAGS += -g -O0
else
  KOS_CFLAGS += -O2
endif

# a failed recipe (a bad patch, the link-map check) must not leave a fresh-looking target behind (as g2bench, d4231ce1)
.DELETE_ON_ERROR:

all: rm-elf $(TARGET)

tcpfix:
	$(MAKE) TCPFIX=1

include $(KOS_BASE)/Makefile.rules

clean:
	-rm -rf build/kos build/kos-fix
	-rm -f netgd-t2.elf netgd-t2-tcpfix.elf

rm-elf:
	-rm -f $(TARGET)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: src/%.c | $(OBJDIR)
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(OBJDIR)/menu_%.o: $(MENU)/src/%.c | $(OBJDIR)
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(OBJDIR)/recv_%.o: $(RECV)/src/%.c | $(OBJDIR)
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(OBJDIR)/ff_%.o: $(FATFS)/%.c | $(OBJDIR)
	kos-cc $(KOS_CFLAGS) -c $< -o $@

$(OBJDIR)/net_tcp_fix.c: $(KOS_NET_SRC)/net_tcp.c $(PATCH) | $(OBJDIR)
	tr -d '\r' < $(KOS_NET_SRC)/net_tcp.c > $(OBJDIR)/net_tcp.orig.c
	tr -d '\r' < $(PATCH) > $(OBJDIR)/fix.patch
	patch -s -o $@ $(OBJDIR)/net_tcp.orig.c $(OBJDIR)/fix.patch

$(OBJDIR)/net_tcp_fix.o: $(OBJDIR)/net_tcp_fix.c
	kos-cc $(KOS_CFLAGS) -I$(KOS_NET_SRC) -c $< -o $@

# the headers are not tracked per object: a header change needs a make clean (CI always builds from clean)
$(OBJS): $(wildcard src/*.h) $(wildcard $(MENU)/src/*.h) $(wildcard $(RECV)/src/*.h)

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS) -lm $(MAP)
ifeq ($(TCPFIX),1)
	@grep -q 'net_tcp_fix_wnd_updates' $(OBJDIR)/t2.map || { echo 'patched net_tcp.c not linked'; rm -f $(TARGET); exit 1; }
	@if grep -q 'libkallisti.a(net_tcp.o)' $(OBJDIR)/t2.map; then echo "KOS's own net_tcp.o was linked too"; rm -f $(TARGET); exit 1; fi
	@echo 'tcpfix: patched net_tcp.c linked, libkallisti.a(net_tcp.o) not pulled'
endif

# make run DC_IP=192.168.10.2
run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

.PHONY: all tcpfix clean rm-elf run
