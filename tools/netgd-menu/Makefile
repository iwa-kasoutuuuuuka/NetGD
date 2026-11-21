# netgd-menu - the NetGD menu, KallistiOS build (Dreamcast).  Compile-checked by CI (.github/workflows/kos.yaml);
# there is no KOS toolchain on the development PC.  Build inside the DreamSDK shell or any KOS environment:
#     make             (release)         make DEBUG=1   (-g -O0)
#     make clean
#     make cdi         netgd-menu.cdi for the SD card folder 01 (needs makeip, scramble, mkisofs, cdi4dc; see README_ja.md)
# build.ps1 runs this Makefile from PowerShell (DreamSDK).  Host tests: Makefile.host / test/run_tests.py.
# Output: netgd-menu.elf  (dc-tool-ip -x netgd-menu.elf for development)
#
# The object files go to build/kos/ (never next to the sources, so the netgd-recv build is not disturbed):
#   src/X.c -> build/kos/X.o      ../netgd-recv/src/X.c -> build/kos/recv_X.o      FatFs -> build/kos/ff_X.o
# Do not pass a -D with a quoted string: it does not survive kos-cc (see tools/netgd-recv/README_ja.md).

TARGET = netgd-menu.elf

RECV   = ../netgd-recv
FATFS  = $(RECV)/third_party/fatfs
OBJDIR = build/kos

MENU_OBJS = $(addprefix $(OBJDIR)/, nm_text.o nm_slot.o nm_cfgparse.o nm_ipbin.o nm_list.o nm_input.o nm_screen.o \
              nm_scan.o nm_launch.o nm_ui.o ngtp_frame.o ngtp_srv.o main_kos.o plat_kos.o net_kos.o)
RECV_OBJS = $(addprefix $(OBJDIR)/recv_, crc32.o manifest.o diskio.o ngblk.o ngfs.o fwctl.o recv_core.o dev_kos.o fwxfer_kos.o)
FATFS_OBJS = $(addprefix $(OBJDIR)/ff_, ff.o ffunicode.o ffsystem.o)
OBJS = $(MENU_OBJS) $(RECV_OBJS) $(FATFS_OBJS)

KOS_CFLAGS += -I src -I $(RECV)/src -I $(FATFS) -std=gnu11 -Wall -Wno-unused-function -Wno-format-truncation
# a different default hand-off: make NM_HANDOFF_DEFAULT=1 (0 = BIOS menu, 1 = restart); NM_SHOW_SLOT1=1 lists folder 01 too
ifneq ($(NM_HANDOFF_DEFAULT),)
  KOS_CFLAGS += -DNM_HANDOFF_DEFAULT=$(NM_HANDOFF_DEFAULT)
endif
ifneq ($(NM_SHOW_SLOT1),)
  KOS_CFLAGS += -DNM_SHOW_SLOT1=$(NM_SHOW_SLOT1)
endif
ifeq ($(DEBUG),1)
  KOS_CFLAGS += -g -O0
else
  KOS_CFLAGS += -O2
endif

all: rm-elf $(TARGET)

include $(KOS_BASE)/Makefile.rules

clean: rm-elf
	-rm -rf $(OBJDIR)

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

# the headers are not tracked per object: a header change needs a make clean (CI always builds from clean)
$(OBJS): $(wildcard src/*.h) $(wildcard $(RECV)/src/*.h)

$(TARGET): $(OBJS)
	kos-cc -o $(TARGET) $(OBJS) -lm

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

.PHONY: all clean rm-elf run cdi
