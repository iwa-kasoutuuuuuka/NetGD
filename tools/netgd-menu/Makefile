# netgd-menu - the NetGD menu, KallistiOS build (Dreamcast).  Compile-checked by CI (.github/workflows/kos.yaml);
# there is no KOS toolchain on the development PC.  Build inside the DreamSDK shell or any KOS environment:
#     make             (release)         make DEBUG=1   (-g -O0)
#     make tcpfix      netgd-menu-tcpfix.elf: KOS's net_tcp.c replaced by a patched copy (../kos-patches), NGTP window
#                      16 KiB x 1 for now, like the default build (64 KiB x 2 is the shape that hung the real Dreamcast in
#                      g2netbench p4, cause unknown).  make NM_WANT=65536 NM_DEPTH=2 asks for it.  As tools/netgd-t2.
#     make NM_IP=0xC0A80A02 NM_RCVBUF=20480   the static IP (no DHCP) and the TCP receive window (numbers only)
#     make clean
#     make cdi         netgd-menu.cdi for the SD card folder 01 (needs makeip, scramble, mkisofs, cdi4dc; see README_ja.md)
# build.ps1 runs this Makefile from PowerShell (DreamSDK).  Host tests: Makefile.host / test/run_tests.py.
# Output: netgd-menu.elf  (dc-tool-ip -x netgd-menu.elf for development)
#
# The object files go to build/kos/ (never next to the sources, so the netgd-recv build is not disturbed):
#   src/X.c -> build/kos/X.o      ../netgd-recv/src/X.c -> build/kos/recv_X.o      FatFs -> build/kos/ff_X.o
# Do not pass a -D with a quoted string: it does not survive kos-cc (see tools/netgd-recv/README_ja.md).

RECV   = ../netgd-recv
FATFS  = $(RECV)/third_party/fatfs
PATCH  = ../kos-patches/net_tcp-rx-fix.patch
KOS_NET_SRC = $(KOS_BASE)/kernel/net

# DreamSDK is not modified: for tcpfix, $(KOS_BASE)/kernel/net/net_tcp.c is copied, patched and compiled here, and the link map
# is checked (the patched copy is in, libkallisti.a(net_tcp.o) is not), exactly as tools/netgd-t2/Makefile.
ifeq ($(TCPFIX),1)
  TARGET = netgd-menu-tcpfix.elf
  OBJDIR = build/kos-fix
  KOS_CFLAGS += -DNM_TCPFIX=1
  FIX_OBJS = $(OBJDIR)/net_tcp_fix.o
  MAP = -Wl,-Map,$(OBJDIR)/menu.map
else
  TARGET = netgd-menu.elf
  OBJDIR = build/kos
  FIX_OBJS =
  MAP =
endif

MENU_OBJS = $(addprefix $(OBJDIR)/, nm_text.o nm_slot.o nm_cfgparse.o nm_ipbin.o nm_list.o nm_input.o nm_screen.o \
              nm_scan.o nm_launch.o nm_disc.o nm_art.o nm_ui.o ngtp_frame.o ngtp_srv.o main_kos.o plat_kos.o net_kos.o nm_netup.o)
RECV_OBJS = $(addprefix $(OBJDIR)/recv_, crc32.o manifest.o diskio.o ngblk.o ngfs.o fwctl.o recv_core.o dev_kos.o fwxfer_kos.o)
FATFS_OBJS = $(addprefix $(OBJDIR)/ff_, ff.o ffunicode.o ffsystem.o)
OBJS = $(MENU_OBJS) $(RECV_OBJS) $(FATFS_OBJS) $(FIX_OBJS)

KOS_CFLAGS += -I src -I $(RECV)/src -I $(FATFS) -std=gnu11 -Wall -Wno-unused-function -Wno-format-truncation
# a different default hand-off: make NM_HANDOFF_DEFAULT=1 (0 = BIOS menu, 1 = restart); NM_SHOW_SLOT1=1 lists folder 01 too
ifneq ($(NM_HANDOFF_DEFAULT),)
  KOS_CFLAGS += -DNM_HANDOFF_DEFAULT=$(NM_HANDOFF_DEFAULT)
endif
ifneq ($(NM_SHOW_SLOT1),)
  KOS_CFLAGS += -DNM_SHOW_SLOT1=$(NM_SHOW_SLOT1)
endif
# the hand-off variant (R104 finding 3; hardware test H2): make NM_HANDOFF_TUR=1 polls TEST UNIT READY before the hand-off;
# the default (0) hands over right after `52 82` succeeded
ifneq ($(NM_HANDOFF_TUR),)
  KOS_CFLAGS += -DNM_HANDOFF_TUR=$(NM_HANDOFF_TUR)
endif
ifneq ($(NM_IP),)
  KOS_CFLAGS += -DNM_NET_STATIC_IP=$(NM_IP)
endif
ifneq ($(NM_RCVBUF),)
  KOS_CFLAGS += -DNM_RCVBUF=$(NM_RCVBUF)
endif
# the window override is for tcpfix builds only: on stock KOS TCP a larger window stalls (R178)
ifneq ($(NM_WANT)$(NM_DEPTH),)
ifeq ($(TCPFIX),1)
ifneq ($(NM_WANT),)
  KOS_CFLAGS += -DNM_WANT=$(NM_WANT)
endif
ifneq ($(NM_DEPTH),)
  KOS_CFLAGS += -DNM_DEPTH=$(NM_DEPTH)
endif
else
$(info NM_WANT / NM_DEPTH ignored: this is the stock TCP build, which stays at 16 KiB x 1)
endif
endif
ifeq ($(DEBUG),1)
  KOS_CFLAGS += -g -O0
else
  KOS_CFLAGS += -O2
endif

# a failed recipe (a bad patch, the link-map check) must not leave a fresh-looking target behind (as netgd-t2)
.DELETE_ON_ERROR:

all: rm-elf $(TARGET)

tcpfix:
	$(MAKE) TCPFIX=1

include $(KOS_BASE)/Makefile.rules

clean:
	-rm -rf build/kos build/kos-fix
	-rm -f netgd-menu.elf netgd-menu-tcpfix.elf

rm-elf:
	-rm -f $(TARGET)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: src/%.c | $(OBJDIR)
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
$(OBJS): $(wildcard src/*.h) $(wildcard $(RECV)/src/*.h)

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS) -lm $(MAP)
ifeq ($(TCPFIX),1)
	@grep -q 'net_tcp_fix_wnd_updates' $(OBJDIR)/menu.map || { echo 'patched net_tcp.c not linked'; rm -f $(TARGET); exit 1; }
	@if grep -q 'libkallisti.a(net_tcp.o)' $(OBJDIR)/menu.map; then echo "KOS's own net_tcp.o was linked too"; rm -f $(TARGET); exit 1; fi
	@echo 'tcpfix: patched net_tcp.c linked, libkallisti.a(net_tcp.o) not pulled'
endif

# make run DC_IP=192.168.1.200
run: $(TARGET)
	dc-tool-ip -t $(DC_IP) -x $(TARGET)

# ---- netgd-menu.cdi (NOT built by CI).  The tools are variables so that another one can be named. ----
OBJCOPY  ?= sh-elf-objcopy
SCRAMBLE ?= scramble
MAKEIP   ?= makeip
MKISOFS  ?= mkisofs
CDI4DC   ?= cdi4dc
CDIDIR    = build/cdi

cdi: $(TARGET) ip.txt
	rm -rf $(CDIDIR)
	mkdir -p $(CDIDIR)/root
	$(OBJCOPY) -R .stack -O binary $(TARGET) $(CDIDIR)/netgd-menu.bin
	$(SCRAMBLE) $(CDIDIR)/netgd-menu.bin $(CDIDIR)/root/1ST_READ.BIN
	$(MAKEIP) ip.txt $(CDIDIR)/IP.BIN
	$(MKISOFS) -C 0,11702 -V NETGD_MENU -G $(CDIDIR)/IP.BIN -l -o $(CDIDIR)/netgd-menu.iso $(CDIDIR)/root
	$(CDI4DC) $(CDIDIR)/netgd-menu.iso netgd-menu.cdi
	@echo "Built netgd-menu.cdi: put it alone in the SD card folder 01"

.PHONY: all tcpfix clean rm-elf run cdi
