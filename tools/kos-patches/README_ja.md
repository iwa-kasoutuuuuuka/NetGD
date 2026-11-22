# KOS へのパッチ（DreamSDK は書き換えない）

## `net_tcp-rx-fix.patch`（KallistiOS 2.3.0 の `kernel/net/net_tcp.c`）

KOS の TCP の受信の 2 つの不具合を直す（実機の症状は `docs/research/LOG.md` R175・R178、分析は `docs/research/g2-transport.md` §1・§3）。

1. **順番違いのセグメントでストリームが壊れる**：`process_pkt` は、セグメントの先頭が窓の中にあれば `seq == rcv.nxt` を確かめずに受信バッファの末尾へ足す。BBA の受信リング（16 KiB）で 1 個落ちると、その先のセグメントが続きとして積まれる。→ `seq == rcv.nxt` のものだけ積み、先へ飛んだものは捨てて重複 ACK を返す。`rcv.nxt` より前から始まる再送は持っている部分を切り捨てる（受け入れの条件を RFC 793 のとおり「窓と重なれば可」に広げる）。
2. **フレームが窓より大きいと止まる**：`net_tcp_recvfrom` は窓（`rcv.wnd`）を広げるだけで ACK を送らないので、窓が 0 になった送り手は持続タイマーを待つ。→ `recv` で窓が受信バッファの半分を越えたら窓の更新の ACK を 1 個送る（RFC 1122 4.2.3.3）。

働いた回数を `net_tcp_fix_ooo_dropped`・`net_tcp_fix_trimmed`・`net_tcp_fix_wnd_updates`（`volatile uint32_t`、外から `extern` で読める）で数える。

### 使い方（アプリに一緒にリンクする）

KOS の `net_tcp.o` の外向きの名前は `net_tcp_init`・`net_tcp_shutdown` の 2 つだけなので、直した版をアプリのオブジェクトに入れると、リンカは `libkallisti.a` の `net_tcp.o` を取り出さない（アプリのオブジェクトは `-lkallisti` より前）。手順の例は `tools/g2bench/Makefile` の `net-tcpfix`：

```make
build-fix/net_tcp_fix.c: $(KOS_BASE)/kernel/net/net_tcp.c ../kos-patches/net_tcp-rx-fix.patch
	mkdir -p build-fix
	tr -d '\r' < $(KOS_BASE)/kernel/net/net_tcp.c > build-fix/net_tcp.orig.c
	tr -d '\r' < ../kos-patches/net_tcp-rx-fix.patch > build-fix/fix.patch
	patch -s -o $@ build-fix/net_tcp.orig.c build-fix/fix.patch
net_tcp_fix.o: build-fix/net_tcp_fix.c
	kos-cc $(KOS_CFLAGS) -I$(KOS_BASE)/kernel/net -c $< -o $@
```

リンクのマップ（`-Wl,-Map,...`）に `libkallisti.a(net_tcp.o)` が無く、`net_tcp_fix_wnd_updates` があることで、直した版だけが入ったと確かめられる（g2bench の Makefile はこれを確かめ、違えばビルドを止める）。

- DreamSDK の `net_tcp.c` は読むだけ（写してから当てる）。DreamSDK の CRLF は落としてから当てるので、パッチは LF のまま置く（`.gitattributes`）。
- KOS を上げたら当たるか確かめる。当たらなければ `patch` が止まる（黙って古い版が入ることはない）。
- KallistiOS の利用条件（BSD 系）はそのまま：生成した `net_tcp_fix.c` は元の著作権表示を残している。
