# netgd-menu（NetGD 専用メニュー）

SD カードのフォルダ `01` に置く Dreamcast 用メニュー。NetGD のファームにだけ対応する。設計は `docs/design/menu-app.md`。

> **重要：実機では一度も動かしていない。**
> ホスト（PC の gcc）での試験だけが済んでいる（下の「テスト」）。KOS 用の部分（`src/main_kos.c` `src/plat_kos.c` `src/net_kos.c`、それと `tools/netgd-recv` の `dev_kos.c` `fwxfer_kos.c`）は **この PC にツールチェーンが無く、コンパイルは CI（`.github/workflows/kos.yaml`、KOS 2.3.0）だけ。実機では一度も動かしていない**。最初の実機検証では、下の「初回確認リスト」を上から順に確かめること。
> 純正 GDEMU には対応しない。起動時にまず `52 00`（GDEMU_VERSION、純正の記録と同じ CDB）だけを送り、**NetGD の 5.20.0 と答えたときだけ** `E0 03`（VERSION）を送る（netgd-t2 と同じ `fw_probe_netgd`）。それ以外（純正の 5.20.5 など）には `E0` を 1 個も送らず、SD にも触らず、理由（例「GDEMU 5.20.5 is not NetGD: no NetGD command sent.」）を出して START で BIOS に戻るだけ。

## 何をするか

1. SD カードのルートを **メニュー自身が読み**（セカンダリ IDE ディスク + FatFs、読み取り専用）、番号フォルダ（`02`、`03`、…）と名前フォルダ（`netgd-send --name` で書いたもの）を一覧にする。フォルダ `01`（このメニュー）は出さない。
2. 十字キーで選び、**A** で起動。`0x52`（番号指定）または `E0 05`/`E0 02`（名前指定）でファームにマウントさせ、READY を待ってから BIOS に戻る。BIOS の画面で「プレイ」を選ぶ（既定）。情報画面（X）で「本体を再起動する」方式に切り替えられる。
3. タイトルは先に見つかったもの：`GAME.CFG` の `title`、`NAME.TXT` の 1 行目、GDI の高密度トラックの IP.BIN、フォルダ名。ASCII 以外は `?`。

| ボタン | 一覧 | 取り込み（手順 6 以降） |
|--------|------|-------------------------|
| 上下（アナログも） | 1 件移動（押し続けで連続） | — |
| 左右、L/R | 1 ページ（15 件） | — |
| A | 起動 | 確認画面で許可（1 秒押し続け） |
| X | 情報画面 | — |
| Y | 取り込み画面へ | — |
| START | BIOS メニューへ（確認あり） | — |

## ビルド

DreamSDK のシェル（`KOS_BASE` 設定済み、KOS は 2.3.0 のユーザーフォーク）：

```
cd tools/netgd-menu
make                 # netgd-menu.elf（KOS の TCP のまま：NGTP の窓 16 KiB × 1）
make tcpfix          # netgd-menu-tcpfix.elf（直した net_tcp.c を一緒にリンク：当面 16 KiB × 1）
make cdi             # netgd-menu.cdi（SD のフォルダ 01 用。CI では作らない）
```

PowerShell からは `.\build.ps1 [-Debug] [-TcpFix] [-Cdi] [-Make 'NM_IP=0x...'] -BbaTools <Dreamcast-Dev-Tools-for-BBA のフォルダ>`。

- **B ボタン**：B は「離された」のを見てから数える（`../netgd-recv/src/ng_btnguard.h`。実機で B が押されっぱなしに見えるパッドがあった）。B が固まっている間は、情報画面・BIOS 確認・取り込み画面（待機中）・Drive not ready のダイアログは START で戻れ、画面にそう出る。転送中は PC 側から止める。A と START も離されたのを見てから数える。保護フォルダの強制の確認は、質問が出てから A をいったん離し、1 秒長押しすると許可になる（固まった A や全ボタン押下に見える状態では許可されない）。それ以外の正常なパッドの操作は変わらない。
- オブジェクトは `build/kos/`（tcpfix は `build/kos-fix/`）に出る（`tools/netgd-recv/src/*.c` と FatFs のオブジェクトもここ。netgd-recv 側のビルドには触らない）。
- **NGTP の窓**（WANT の大きさ × 同時数）：既定は **16 KiB × 1**。KOS 2.3 の TCP は窓より大きいフレームでほぼ止まる（R178：32〜64 KiB で 4〜6 KB/s）ため。`make tcpfix` も当面 **16 KiB × 1**（64 KiB × 2 は g2netbench p4 と同じ形で実機の Dreamcast が 2 回止まった。原因は未解明で、この既定は止まる形を避けるだけ。`make tcpfix NM_WANT=65536 NM_DEPTH=2` で明示できる（素の TCP のビルドでは無視）。`tools/netgd-t2` と同じ仕組み：`$(KOS_BASE)/kernel/net/net_tcp.c` を写して `../kos-patches/net_tcp-rx-fix.patch` を当て、ここでコンパイルしてリンク。リンクマップで直した版が入り KOS の `net_tcp.o` が入っていないことを確かめる。DreamSDK は変更しない）。受信バッファ（= TCP の窓）は両方とも listen の前に 20480 B（`make NM_RCVBUF=…` で変更）。取り込み画面の 4 行目に `window 16 KiB x 1 (stock tcp)` のように出る。
- `make NM_IP=0xC0A80A02`：固定 IP（既定 192.168.10.2）。数値だけ（引用符つきの `-D` は kos-cc を通らない）。
- `make cdi` が使うツール（`sh-elf-objcopy`、`scramble`、`makeip`、`mkisofs`、`cdi4dc`）は変数で差し替えられる（`MKISOFS=... make cdi`）。**`mkisofs`・`cdi4dc` が DreamSDK に入っているかは未確認**（無ければ `mkdcdisc` に替える）。`ip.txt` の書式も実機の DreamSDK の `makeip` で確かめること。
- `make NM_HANDOFF_DEFAULT=1`：既定の渡し方を再起動にする（0 = BIOS メニュー）。`make NM_SHOW_SLOT1=1`：フォルダ `01` も一覧に出す。
- `make NM_HANDOFF_TUR=1`：BIOS に渡す前に TEST UNIT READY を 20 ms ごとに送って待つ（2 回続けて READY で渡す。実機試験 H2 の変種 B）。**既定（0）は送らず、`52 82` が成功した時点で BIOS に渡す**（変種 A）。ファームは UNIT ATTENTION を BIOS に残す設計なので、メニューの TUR がそれを消費すると、BIOS が古い目次のままになるおそれがある（R104 の指摘 3、推測）。

開発中の実行：`dc-tool-ip -t <IP> -x netgd-menu.elf`（一覧と起動だけ。dcload-ip が BBA を使っているので、取り込みは無効：Y を押すと「Started from dcload: use netgd-recv」と出る）。

## SD カードへの置き方

フォルダ `01` に `netgd-menu.cdi` を入れる（他の `.gdi`／`.cdi` は置かない）。ゲームは `02`、`03`、…（2〜4 桁の数字）に置く。`GDEMU.INI` の `reset_goto=1` は、再起動方式を使うかどうか（H3）が分かるまで、渡し方を BIOS（既定）にしたときだけにする。

## スタックの最高水位を読む（ファームの機能 `stack-hw`、実機未検証）

`docs/PLAN.md` A-1 の実測用。ファームを cargo の機能 `stack-hw` 付きでビルドしたときだけ使える（既定のビルドには入らない。入れないときのイメージは従来と同じバイト列）。

- ファーム：`main` がクロックの初期化の直後に、空いているスタック（`__sheap` から、そのときの SP の 32 バイト下まで）を `0xC5C5C5C5` で塗る。
  `0xE0` のサブコマンド 9（STACK、読むだけ）が、下から見て最初に塗りが消えた語を探し、20 バイトの応答を返す：
  `"NGSK"`、形式 1、予約 3 バイト、`reserved`（スタック領域の大きさ）、`used`（リセット以降の最高水位）、`unpainted`（塗らなかった上端。`used` はこれより小さくならない）。すべて u32 リトルエンディアン、バイト数。
  VERSION の機能バイトの bit 3（`0x08`）が「STACK に答える」の印。形式の定義は `common/stackmark/src/lib.rs`。
- メニュー：情報画面（**X**）に入るたびに 1 回だけ読み、`Stack         4816 of 7128 bytes used` の行（9 行目）を出す。`used` が `reserved` に届いていれば末尾に `OVERRUN`（スタックが静的変数の領域まで来た）。bit 3 の無いファームには送らず、行も出ない。

実機での手順：

1. ファームをビルドして書き込む（2E の常駐部レイアウトの例）：
   `DEFMT_LOG=off cargo build --profile release-z --features sam3u2e,resident,sd-fast,stack-hw`（4E は `cargo build --release --features stack-hw`）。封印・書き込みはいつもどおり。
2. 電源を入れ、測りたい操作をする（ゲームの起動、ディスクの切り替え、CDDA、GDI の多いフォルダなど）。値は **MCU のリセット（電源投入）からの最大**で、電源を切ると消える。
3. 電源を切らずにメニュー（フォルダ `01`）へ戻る（`reset_goto=1` で本体のリセット、など）。**本体のリセットボタンで MCU までリセットされるかは未確認**：戻った直後に値が小さければ、リセットされている。
4. 一覧で **X**。情報画面の `Stack` の行を読む。機能の欄が `features 0x0F` なら `stack-hw` のファーム。
5. BBA があれば `netgd-recv` でも読める：書き込みセッションの開始時、`NGD:HELLO` の次の行に `NGD:STACK fmt=1 reserved=… used=… free=… unpainted=…` が出る（`dc_auto` のログと `result.ngr`）。
6. defmt のログを有効にしたビルド（4E、プローブ）では、STACK を読むたびに `stack high-water … of … bytes` がログに出る。

読み方の注意：塗りの方式なので、書き込まずに確保しただけのスタック（埋めなかったバッファ）は数えない（実際より小さく出ることがある）。静的見積もり（LOG R127・R131）の最深経路 5,200 B（release）／4,800 B（release-z resident）と比べる。

## テスト（この PC でできるもの）

```
python tools/netgd-menu/test/run_tests.py            # ホストでのメニュー中核の試験（要 gcc、40 件。取り込みは 127.0.0.1 のソケット）
python tools/netgd-recv/test/run_tests.py            # 受信コアの試験（既存 + 軽いマウントとメニュー用コマンド）
```

`menu_host`（`test/menu_host.c`）はメニューの中核を仮想時計で動かす。`menu_host ui <img> <sectors> --script "scan D A t:2000 dump"` のように、ボタンの列を与えて画面とファームへのパケットを見られる。カードのイメージは `tools/netgd-recv` の `test_host mkimage`／`addfile`／`mkdirs` で作る（`run_tests.py` が両方をビルドする）。

実装したもの：一覧・走査・タイトル・起動（0x52／名前）・READY 待ち・情報画面・BIOS への渡し、BBA からの取り込み（NGTP 版 1）。**すべて実機未検証。** UDP の探索（設計の手順 9）、箱絵（設計の手順 11）。無いもの：ファーム更新の統合、実機の数字で決めるもの（設計の手順 10・12、任意）。

## BBA からの取り込み（実機未検証）

1. 一覧で **Y**。メニューが BBA を**手で**起こし（DHCP なし。下の「BBA の立ち上げ」）、IP アドレス・ポート 20039・4 桁の合言葉を出して待つ。この間は SD カードの読み取り専用マウントを外している（書き込みセッションと同時には持たない）。
2. PC：`python tools/netgd-send/netgd-send.py push game.gdi --host <IP> --code <合言葉> --slot auto`（詳しくは `tools/netgd-send/README_ja.md`）。最初の要求でファームの書き込みセッションが開く（メニューのイメージは排出される。メニューは RAM 上で動き続ける）。
3. 進捗は本体の画面と PC の両方に出る（同じ `NGD:` 行）。**B**：待機中は取り込み画面を出る、転送中は安全に止める（PC は終了コード 2。同じ `push` で続きから再開）。保護フォルダ（`GDEMU.CFG` の既定・`01`）への上書きは、PC の `--force` と本体で **A を 1 秒押し続ける** の両方が要る（B か 30 秒で拒否）。
4. PC が `FINISH` を送る、B で出る、または要求が 5 分来ないとセッションを閉じ（既定のイメージが戻る）、一覧を読み直す。新しいゲームが出る。閉じられなかったときは「Could not close the session: reset the console」と出て止まる（リセットでファームが戻す）。

構成：`src/ngtp.h`・`ngtp_frame.c`（フレーム）、`ngtp_srv.c`（サーバ。受信コア `tools/netgd-recv/src/recv_core.c` を変更せず呼ぶ）、`src/nm_net.h`（ネットワークの境界。KOS は `net_kos.c`、ホスト試験は `test/net_host.c` の 127.0.0.1 のソケット）。`net_kos.c` と `nm_netup.c` は **netgd-t2 と共用**（t2 の Makefile が `menu_net_kos.o`・`menu_nm_netup.o` としてコンパイルする。t2 の `src/net_kos.c` は無くなった）。

### BBA の立ち上げ（DHCP なし、実機未検証）

以前の `net_init(0)`（DHCP を待つ）は PC と Dreamcast を直結したケーブルでは返ってこない。netgd-t2・`tools/g2bench` と同じ手順に替えた（手順と時間の上限は `src/nm_netup.c`、KOS の呼び出しは `src/net_kos.c`）：

1. IP を決める：dcload-ip から起動したときはその IP と PC の IP、それ以外は固定 IP（既定 **192.168.10.2**、`make NM_IP=…`）。ネットマスク 255.255.255.0、PC（ゲートウェイ）は x.y.z.1。**PC 側は 192.168.10.1/24 などを手で設定する**。
2. `eth_init()` でドライバを登録し、すべての netif に IP を入れる（netif が 0 個 =「BBA not found」）。
3. `net_init(0)` をスレッドで動かし **30 秒まで**待つ（BBA のドライバ自身がリンクを 10 秒待つ）。`-1` が返ったら「No link: check the cable and restart the console」で**終わり**（Y の表示も消え、Y を押してもこのメッセージが出るだけ）。KOS 2.3 の `bba_if_start()` はリンク待ちに失敗しても rx スレッドを止めず `bba_rx_thread` を残すので、2 回目の `net_init()` は `assert(bba_rx_thread == NULL)` で abort する（`nm_netup.h` の `init_again`）。ケーブルを挿してから本体を再起動する。30 秒で返らなければ「No network link yet (Y: wait again)」：次の Y では 2 つ目の `net_init` を起こさず、同じスレッドをもう 30 秒待つ（それでも返らなければ「Network start hangs: restart the console」）。
4. 既定のデバイスに IP を入れ直す（ドライバが flashrom の ISP 設定を読むことがあるため）。PC の ARP を **8 秒まで**（答えがなくても失敗にしない）。
5. 各手順は「Starting network ...」の画面の 7 行目から 1 行ずつ出る。どの待ちにも上限があり、無限に待つループは無い。

ホスト試験（`menu_host selftest` の U11）は、偽の KOS 操作と仮想時計で手順の順番・時間の上限・エラー文・やり直しを確かめる。KOS の呼び出し自体（`eth_init`・`net_init` のスレッド・`net_arp_lookup`）は netgd-t2 の実績（R175・R178 の g2netbench）と同じだが、**`net_init` が失敗した後の 2 回目の `net_init` は実機で試していない**。

## 初回確認リスト（実機、上から順に）

最初は**捨てても困らない SD カード**で。

1. 起動：`netgd-menu.cdi` を SD のフォルダ `01` に入れて電源を入れる。BIOS からメニューが起動するか（H1：`INIT_CDROM` を外した KOS アプリがイメージから起動でき、`fw_xfer`（生 ATAPI）と `g1ata` が両方動くか）。
2. 画面：53 桁が収まるか。PAL（50 Hz）・VGA でも（H10：`plat_kos.c` の `X0`・`Y0` で余白を調整）。右上に `fw 5.20.0` のようにファームの版が出るか。
3. 一覧：番号フォルダが出るか。タイトルが正しいか。100 本で走査にかかる時間（H5）。
4. 起動：A で `0x52` が通り、READY になり、BIOS に戻って「プレイ」で起動するか（H2：BIOS が新しいディスクを認識するか。UNIT ATTENTION の扱い）。
   - **H2 の 2 変種（両方を試す）**：変種 A（既定、`make`）＝ `52 82` の成功の直後に BIOS へ渡す。変種 B ＝ `make NM_HANDOFF_TUR=1`（TEST UNIT READY で READY を確かめてから渡す。待ちが長引くと「Drive not ready」の画面が出る）。どちらで BIOS が新しいディスクを認識し、「プレイ」で起動するかを記録する。A で認識しなければ B、B で古い目次のままなら A が原因。ホスト試験は両方の組み立てで通る（`test/run_tests.py`）。いずれも実機は未確認。
5. 再起動方式（情報画面で切り替え）：`arch_reboot()` が G1 のハードリセットを出すか。`reset_goto=1` のカードでメニューに戻ってしまわないか（H3、ファームのログ「hard reset」）。
6. イメージをマウントしたまま、セカンダリの読み出しが正しく通るか（H4）。
7. 同じ番号のフォルダが 2 つあるときにファームがどちらを選ぶか（H8）。
8. `reset_goto`・`open_time`・`detect_time` の表示が `GDEMU.INI` と合っているか。

9. 取り込み（H6、H7、H9）：BBA なしで Y を押して「BBA not found」がすぐ出るか。BBA ありでケーブルを抜いたまま Y：約 10 秒で「No link: check the cable and restart the console」が出て、もう一度 Y を押しても落ちない（net_init を再び呼ばない）か。挿して再起動すれば通るか。取り込み画面を何度も出入りしても空きメモリが減らないか（リッスンソケットは閉じずに使い回す）。直結ケーブル（PC 192.168.10.1/24）で 192.168.10.2 が出て、ARP の行が「PC 192.168.10.1 answered ARP」になるか。`netgd-send push` が最後まで通り、実効速度はどれくらいか（既定 16 KiB × 1 で約 2 MB/s の見込み（R178）、tcpfix 版（既定は同じ 16 KiB × 1）と比べる）。純正 GDEMU で起動したとき `52 00` だけで止まるか（ファームのログ、または gdprobe の記録と比べる）。セッションを閉じた後に `52 82` がすぐ通るか。`tools/netgd-recv/README_ja.md` の初回確認リスト（DMA・FLUSH・FNV 照合・マウント時間）も同じ経路。

KOS 2.3.0 のヘッダで確かめること（CI で通れば確認済み）：`arch_menu()`／`arch_reboot()`（K1）、`bfont` の色設定（netgd-fwupd が使用）、`maple`（netgd-recv が使用）、ネットワーク：`net_init`・`net_default_dev->ip_addr`・TCP の `listen`／`accept`・`O_NONBLOCK`（K2〜K4、`net_kos.c`）。

## 実装メモ（設計の任意の手順 9 と 11。設計書は書き換えていない）

### 手順 9：UDP の探索（実機未検証）

- 取り込み画面を開いている間（`NET_IDLE`／`NET_BUSY`）だけ、UDP 20039 で待つ。`net_start` で開き（開けなくても何も出さない。IP を手で指定すればよい）、`nm_net_stop` で閉じる。TCP のリッスンソケットは `nm_net_stop` では閉じず（accept しないだけ）、次の `nm_net_listen` で溜まった接続を捨ててから使い回す：KOS 2.3 の `net_tcp.c` はリッスン中のソケットを閉じるとき `SO_RCVBUF` で確保した受信バッファ（20 KiB）を解放しないため。
- 要求は `NGTP-DISCOVER 1`（後ろに空白・改行が続いてもよい。`NGTP-DISCOVER 10` や別の版には答えない）。答えは `NGTP-HERE 1 name=NetGD app=<版> state=idle|busy`。`busy` は「PC が TCP でつながっている」（もう 1 台は入れない）。**合言葉と TCP のポートは答えに入れない**（ポートは NGTP の 20039 固定。PC は `--port`）。
- 1 フレームに 1 通だけ見る（`nm_disc_service`）。操作の最中（`RECV_BUSY`）は 1 つの操作が終わるまで答えない。そのとき PC は 2 秒で諦めて `--host` を求める。
- 中核は `src/nm_disc.c`（文字列だけ。ソケットは持たない）。境界に `nm_net_udp_listen`／`nm_net_udp_recv`（待っていなければ 0 を返す）／`nm_net_udp_reply`（最後に受けた相手へ）を足した。KOS 側（`net_kos.c`）は `SOCK_DGRAM`・`recvfrom`・`sendto`・`socklen_t`・データグラムソケットの `O_NONBLOCK` が `KOS-API-UNVERIFIED`。

### 手順 11：箱絵（実機未検証）

- **形式**：ゲームのフォルダの `NETGD.ART`（大文字小文字は問わない）。無圧縮。ヘッダ 16 バイト（`"NGAR"`、版 u16 = 1、幅 u16 = 128、高さ u16 = 128、予約 6 バイト（見ない））＋ 128×128 の RGB565（リトルエンディアン、左上から行順）。計 32,784 バイト。デコーダ無し（読んで並べ替えるだけ）。後ろに余分なバイトがあってもよい。
- **無視するもの**（エラーにも一覧の停止にもしない。絵が出ないだけ）：ファイルが無い／短い／ヘッダの魔法数・版・大きさが違う／読み取りエラー。失敗したフォルダは、カーソルが別の所へ動くまで再試行しない。
- **いつ読むか**：カーソルが同じゲームに 300 ms 止まったら（`nm_art.c`、`NM_ART_DEBOUNCE_MS`）、軽いマウントのまま 1 フレームに 4 KiB ずつ読む（全部で約 9 フレーム。1 回の読み取りで画面が止まらない）。完成するまで出さない。カーソルが動く、情報画面・確認・起動・取り込みに移ると、すぐ消す／読みかけを閉じる（起動・取り込み・BIOS への確認の前に `nm_art_cancel`：FatFs のファイルを閉じてからボリュームを外す）。
- **置き場所**：メモリは `nm_ui_t` の中の 32 KiB 1 枚（`nm_art_t`）だけ。画面は既存の 53×20 のマス目の右端の 11 桁×6 行（桁 42〜52、行 2〜7）を空けて、その上に 128×128 を直接描く（`nm_plat_set_art()`。KOS は `vram_s` へ書く）。マス目の層に画像は混ぜない。絵が一度でも出たら、一覧の行は題名 30 桁（絵が無いゲームの上でも。行が伸び縮みしない）に狭め、絵のブロックのマスは空白・通常属性に保つ（ホスト試験が毎回検査：`dirty=0`）。プラットフォームは絵を変えるとき、そのブロックのマスを描き直してから絵を描く（消した絵が残らない）。絵が一度も出ていないとき（`NETGD.ART` が 1 つも無い SD カード）の画面は従来と同じ。
- **PC 側**：`netgd-send push --art` と `netgd-send art`（`tools/netgd-send/README_ja.md`）。絵の変換は PC で行い、Dreamcast は縮小も色変換もしない。
- KOS：新しい API は使っていない（`vram_s`・FatFs・`bfont` は既存の使い方のまま）。`KOS-API-UNVERIFIED` は増えない。
- 実機で見ること：絵が出るまでの時間（32 KiB を 4 KiB ずつ読むので H5 の 1 コマンドの時間次第）、カーソルを速く動かしたときに読みかけで固まらないか、PAL・VGA で右端の 11 桁（132 px）に絵が収まるか（H10）。

## 独立レビュー（R104）の指摘への対応（実機未検証）

- **指摘 1（BBA）**（※ fix/menu-net で手動の立ち上げに置き換え。上の「BBA の立ち上げ」）：`net_kos.c` が `net_init` の前に `bba_init()`（ヘッダがあれば `la_init()`）を呼ぶ。第 2 の Y で、IP が無ければ `net_dhcp_request` をやり直す。DHCP の前に「Starting network ...」を描く。`KOS-API-UNVERIFIED`：`bba_init`・`la_init`（と `__has_include` による有無の判定）・`net_dhcp_request`（引数の有無）・`net_init` の引数。
- **指摘 3（TUR）**：上の `NM_HANDOFF_TUR`（既定 0 = TUR を送らない）と、初回確認リストの H2 の 2 変種を参照。ホスト試験は両方の組み立て（`menu_host`・`menu_host_tur`）で通る。
- **A**：待機中（要求が来ていないとき）に新しい接続が来たら、古い接続を閉じて新しい方に替える（`ngtp_srv.c`）。新しい接続は HELLO からやり直す。書き込みセッションはそのまま。
- **B**：一覧で同じフレームに複数のボタンが来たら、遷移は 1 つだけ（優先順は A・Y・X・START。どれも無いときだけカーソルが動く）。情報画面は B が A より先。
- **C**：セッションを閉じる命令が失敗しても、ファームの状態（`fw_status`）がセッション終了を示していれば、一覧に戻って「The end command failed, but the session is closed」と出す。まだ開いている、または答えが無いときは、これまでどおり「Could not close the session: reset the console」で止まる。
- **D**：「Stop safely」の B で操作が止まったあと、その B が押されたままの間は B を無視する（取り込み画面まで閉じない）。
- **E（直していない）**：`EWOULDBLOCK` のたびに `thd_sleep(1)` するので、速度の上限は約 800 KB/s とみられる。実機（H6）で測ってから、待ちを短くするか `select` 相当にするかを決める。
- **F（直していない）**：番号が重複するフォルダがあるとき、メニューの表示とファームがマウントするゲームが食い違いうる（H8）。実機でファームの選び方を確かめてから、メニュー側を合わせる。
