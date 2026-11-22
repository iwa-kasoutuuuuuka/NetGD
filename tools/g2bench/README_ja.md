# g2bench：`/pc/` の fread 速度・SH-4 の CRC/FNV 速度・スレッド重ね合わせの測定

`docs/research/g2-perf-plan.md` の **M0**。**Dreamcast + BBA + dcload-ip だけ**で動く（NetGD 基板も GDEMU も不要）。
結果は `host\g2bench_<NNN>.txt` に `key=value` で保存される（`<NNN>` は未使用の最小の 001〜）。

## 安全

- **GD-ROM ドライブには一切触らない**：`KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` で cdrom / fs_iso9660 を初期化せず、G1・ATA・cdrom の呼び出しは無い（gdprobe と同じ流儀）。純正 GDEMU 入りの Dreamcast でそのまま使える。
- 書くのは PC 側の `host\g2bench_NNN.txt` だけ。SD・VMU・フラッシュには書かない。

## 測るもの

| 段 | キー（接頭辞） | 内容 |
|---|---|---|
| A | `read_1k_*` `read_4k_*` `read_16k_*` `read_64k_*`、`raw_4k_*` | `/pc/` の `fread` を 1 / 4 / 16 / 64 KiB ずつ（各 **30 秒で打ち切り**、最大 32 MiB）。`raw_4k_*` は stdio を通さない `read()`。各ファイルは `setvbuf(_IONBF)`。`_kbps`（fread の中の時間だけで割る）、`_us_per_call`、`_max_call_us`、`_calls`、`_status`（`ok` / `short_read` / `time_limit`）。64 KiB の回は CRC-32 を検証（`read_64k_crc_ok`）。**各回の開始前に `next_pass=<名前>` を書く**ので、dc-tool-ip がその回で落ちても結果ファイルにどの回かが残る |
| B | `cpu_crc32_64k_*` `cpu_fnv1a_64k_*` `cpu_both_64k_*` | 64 KiB のバッファ（キャッシュ有り）に対する SH-4 の CRC-32 / FNV-1a / 両方の速度。**`tools/netgd-recv/src/crc32.c` をそのままビルドして使う**。各 1.5 秒。`selfcheck_ok` = 標準テストベクタ |
| C | `ov_block_bytes` `ov_blocks`（A で最も速かったブロックサイズ、約 10 秒分。最大 8 MiB）、`ov_cpu_only_*` `ov_read_only_*` `ov_serial_*` `ov_overlap_*` | その量で、CPU（CRC+FNV）のみ / fread のみ / 直列（fread→CRC+FNV）/ 重ね合わせ（**別スレッドが fread、メインスレッドが CRC+FNV**、3 面のリング）。`overlap_status`：`ok`（別スレッドからの `/pc/` が動いた）/ `timeout`（60 秒進まず）/ `thd_create_failed` / `thread_test_running`（結果ファイルがこの値のまま = 別スレッド中に固まった）。`ov_serial_crc_ok` `ov_overlap_crc_ok` = データの正しさ |

読み方：重ね合わせが効くなら `ov_overlap_kbps` が `ov_serial_kbps` を上回り、`ov_read_only` と `ov_cpu_only` の遅い方に近づく。
dcload の syscall が SH-4 を占有する（BBA を CPU がポーリングする）なら、両者はほぼ同じになる。これが知りたい点。
結果ファイルは各段の終わりに書き直すので、途中で止まっても、そこまでの数字と `status=running` が残る。

## 使い方

1. DreamSDK でビルド（`host\g2bench_data.bin` = 32 MiB の決定的データも無ければ作る。Python が要る）：

   ```powershell
   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools
   ```
   データだけ作るなら `python mkdata.py host\g2bench_data.bin 32`。`host\` は git 管理外。
2. GDmenu から DCLOAD-IP BBA を起動してから、`H:\DC\tools\dc-bba-tools` で：

   ```powershell
   .\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu-g2bench\tools\g2bench\profiles\g2bench.ps1
   ```
   （リポジトリの場所に合わせてパスを変える。dc-tool-ip は NetGD リポジトリと同じドライブから動かす = `/pc/` の規則。）
   終わると PostRun が最新の結果ファイルを表示する。**1 回ずつ**実行（`Exclusive`）。
3. 所要時間：B は約 5 秒、A は 5 回 × 最大 30 秒（約 2.5 分）、C は約 10 秒 × 3 回。全体で約 3〜4 分。
4. 大きいブロックで dc-tool-ip が落ちる場合（`error: 10055`）は、上限を下げてビルド：`.\build.ps1 -BbaTools ... -MaxBs 4096`（または `$env:G2BENCH_MAXBS`）。
5. 速度が dc-tool-ip の遅延で決まっているかは、`dc_auto.ps1 -Fast`（`dc-tool-ip -f`）と比べる。

## ホストテスト

`sh tests/run_tests.sh`（gcc と python が要る）。CRC-32/FNV の標準ベクタ、データ生成器、`mkdata.py` との一致、key=value の書き出しと読み取り（コメント・CRLF・不正行・桁あふれ）を確認する。

## ファイル（段 A〜C）

| ファイル | 内容 |
|---|---|
| `main.c` | KOS 側の測定本体 |
| `g2b_core.c/h` | データ生成器、key=value の書き出し・読み取り、速度計算（ホストでテスト） |
| `mkdata.py` | 32 MiB のデータを作る（`g2b_gen_block` と同じ並び） |
| `Makefile` / `build.ps1` / `profiles/g2bench.ps1` | ビルドと dc_auto のプリセット |
| `tests/` | ホストテスト |

## 段 D：KOS の TCP/IP での受信速度（g2netbench）

`menu-app.md` 8 章の **T2（NGTP）** を受け取る側の速さを、SD にも GD-ROM にも触れずに測る。`docs/design/menu-app.md` の試験表 **H6**（「KOS の TCP + BBA の受信速度、`WANT`／`DATA` の往復での実効速度」）にあたる。

> **前提の確認**：今の `netgd-recv`（T1）はソケットを一切使わず、`/pc/`（dcload の syscall）から読むだけ。UDP を使うのは探索（`NGTP-DISCOVER`）だけで、データの経路は **TCP の NGTP（Dreamcast が `WANT`、PC が最大 64 KiB の `DATA`）**。その Dreamcast 側（`ngtp_srv.c`）はまだ無いので、フレームの形・`WANT`／`DATA` の流れ・検査（フレームの CRC-32、ファイルの CRC-32、FNV-1a）をこのツールの中で同じに作って測る。UDP は「独自の UDP 転送を作るなら当てにできる速さと取りこぼし」を見る参考として、バースト送信（段 D の最後）だけ測る。

### KOS のネットワークは dcload-ip と一緒に動くか

動く（KOS 2.3 の `INIT_NET`）。`KOS_INIT_FLAGS(... | INIT_NET)` で起動すると、`arch_init_net_dcload_ip` が dcload-ip から **IP を引き継いで** KOS 自前のスタックで BBA を初期化し、dc-tool-ip とのコンソールと `/pc/` は KOS の `fs_dclsocket`（UDP 経由）に切り替わる。つまり dc_auto で起動でき、printf も `/pc/` も使える。ただし：

- **BBA を KOS が取り直す**ので、以降の `/pc/` は dcload の syscall ではなく `fs_dclsocket`（別経路）になる。`g2bench` の段 A（`/pc/` の速さ）と **同じ値にはならない**（このツールは `/pc/` を使わない）。
- KOS の TCP は RFC 793 のみ（窓の拡大なし、受信バッファは既定 **8 KiB**、`SO_RCVBUF` で最大 65535）。段 D の最初の 2 回（8192 と 65535）で差を見る。
- 結果は `/pc/` に書かず、TCP の `LINE` フレームで PC に返し、PC が `host\g2net_NNN.txt` に保存する（ネットワーク以外の経路に頼らない）。
- 実機で未確認のこと：INIT_NET の初期化が dcload-ip の BBA 状態の上で成功するか、dc-tool-ip のコンソールが途切れないか。動かなければ `[g2net] KOS has no network device` が出る。

### 測るもの

PC の `g2n_send.py` が TCP 20039（NGTP の既定ポート）に接続し、`HELLO` → `PUT`（マニフェストは無視）。Dreamcast は 1 パスにつき最大 32 MiB（**20 秒で打ち切り**）を、次の設定で順に要求する：

| パス | `WANT` の大きさ | 同時に出す `WANT` | 受信バッファ | 検査 |
|---|---|---|---|---|
| p1 | 16 KiB | 1 | 8192（KOS 既定） | あり |
| p2 | 16 KiB | 1 | 65535 | あり |
| p3 | 64 KiB | 1 | 65535 | あり（`menu-app.md` D6 の計画どおり） |
| p4 | 64 KiB | 2（1 つ先読み） | 65535 | あり |
| p5 | 64 KiB | 1 | 65535 | なし（ネットワークだけ） |
| p6 | 64 KiB | 2 | 65535 | なし |

検査 = フレームの CRC-32（`ng_crc32`、ヘッダ + ペイロード）+ ファイルの CRC-32（連鎖）+ FNV-1a（`netgd-recv` が読み戻しで使う）。どのパスも受信したデータを 64 KiB のバッファへ `memcpy`（`ng_src_t.read` と同じ）して捨てる。最後に UDP バースト（1400 バイト × 4000 個を、PC が最大速度 / 2500 pps / 1200 pps で送る）で、届いた数・取りこぼし・KB/s を測る。

キー（`pN_`、N = 1..6）：`want` `depth` `rcvbuf` `checksums`、`bytes` `us` `kbps`、`status`（`ok` / `time_limit` / `timeout` / `bad_*` ...）、`frames`、`recv_calls`、時間の内訳 **`recv_wait_us`（recv の中 = 待ちを含むネットワーク）`send_us` `copy_us` `crc_frame_us` `crc_file_us` `fnv_us` `other_us`**、`max_frame_us`、`crc` `fnv`。PC が配った側の CRC と比べた結果が `pN_pc_crc_ok`（検査ありのパス）、全体が `pc_all_ok`。UDP は `uN_`：`pps` `got` `missing` `late` `bytes` `span_us` `kbps`。

読み方：`recv_wait_us` が支配的なら TCP / BBA / PC 側が律速、`crc_*` `fnv_us` が大きければ SH-4 の計算が律速。p3 と p5 の差が「検査を足したときの落ち」、p3 と p4 の差が「先読みの効き」。KOS のネットワークは別スレッドで動くので、p4 の検査ありで `kbps` が p5 に近づけば計算と受信が重なっている。

### 使い方

1. DreamSDK でビルド（入力ファイルは要らない）：

   ```powershell
   .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Net
   ```
2. GDmenu から DCLOAD-IP BBA を起動してから、`H:\DC\tools\dc-bba-tools` で：

   ```powershell
   .\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu-g2bench\tools\g2bench\profiles\g2netbench.ps1
   ```
   プリセットの `Prepare` が、ELF を送る直前に **PC 側の `g2n_send.py` を自動で起動**する（Python 3 が要る。Dreamcast の IP は dc_auto の `-Ip` / `config\bba.config.ps1`、`$env:G2NET_DC_IP` で上書き可）。送り手は ELF が起動して 20039 を開けるまで最大 900 秒、1 秒おきに接続を試す。終わると `PostRun` が送り手のログと最新の `host\g2net_NNN.txt` を表示する。**1 回ずつ**実行（`g2bench` と同じ `Exclusive`）。
3. 手で動かすなら：ELF を `dc-tool-ip -x g2netbench.elf` で送ってから、`python g2n_send.py --host <Dreamcast の IP> --out-dir host --wait 60`。
4. 所要時間：TCP 6 パス（各 20 秒以内。速ければ 32 MiB で早く終わる）+ UDP 3 回（各約 3〜10 秒）で、**おおよそ 2〜3 分**（最悪で約 2.5 分 + 20 秒）。
5. `dc_auto.ps1 -Fast` は関係しない（`/pc/` を使わない）。ただし KOS の `fs_dclsocket` のコンソール出力（`[g2net] ...` 行）は測定中に出ない（各パスの終わりだけ）。

**ホストテスト**（`sh tests/run_tests.sh` に含まれる）：フレームの組み立て・検査、`WANT`／`DATA` の数え方（深さ 2、末尾の短い断片、順序違い）、UDP パケットと欠落の数え方（C）、送り手とテストパターンの C との一致、偽の Dreamcast を相手にした送り手の通し動作と CRC 不一致の検出（Python）。

### 段 D のファイル

| ファイル | 内容 |
|---|---|
| `main_net.c` | KOS 側（`INIT_NET`、TCP サーバ、パス、UDP 受信） |
| `g2n_core.c/h` | NGTP フレーム、`WANT`／`DATA` の数え方、UDP パケット（ホストでテスト） |
| `g2n_send.py` | PC 側の送り手（`netgd_send.ngtp` のクライアントを使う） |
| `profiles/g2netbench.ps1` | dc_auto のプリセット（送り手を自動起動） |
| `tests/test_net.c` `tests/test_sender.py` | ホストテスト |

## 初回の実機結果と原因の読み取り（2026-11-22）

4 KiB で 38 KB/s（約 105 ms / 呼び出し、最大 165 ms）。16 KiB で dc-tool-ip が `error: 10055`（WSAENOBUFS）を 2 回出して終了。
`H:\DC\tools\dc-bba-tools\build\dcload-ip` のソースから：

- dcload の `read` は 1 回ごとに dc-tool の `dc_read`（`syscalls.c`）を呼び、これは **ファイルの読み出し + 「PC → DC のバイナリ転送」`send_data`**（アップロードと同じ経路）で返す：`CMD_LOADBIN` の往復 → 1440 B の `CMD_PARTBIN` を連続送信（10 パケットごとに 1800 µs 待つ）→ **（`-f` 無しだと）固定 25 ms の待ち**（`PACKET_TIMEOUT/10`）→ `CMD_DONEBIN` の往復（欠落があれば再送と再度の `DONEBIN`）→ `CMD_RETVAL`。つまり 1 呼び出しあたり固定費が「往復 3 回 + 25 ms」あり、ブロックを大きくすれば薄まる（ただし 1 呼び出しは 1 本の同期処理）。
- `send_cmd` は送信エラーを（`WSAEWOULDBLOCK` 以外）致命とみなして `return -1` する（`dc-io.h`）。`dc_read` は `send_data` の失敗を無視して `CMD_RETVAL` も送ろうとし、それも失敗すると `CatchError` で dc-tool 全体が終了する。2 回続く `10055` はこの流れ。
- 25 ms + 往復では 105 ms に足りない分（約 80 ms）はソースだけでは説明できない。DC 側の取りこぼし→再送、Windows の NIC キューなどを疑うが未確認。1 / 4 / 16 KiB と `raw_4k` の呼び出しあたり時間を比べて、固定費と容量比例の分を分ける。

## g2netbench の起動と画面表示（初回の実機で固まった件の対応）

初回は黒い画面のまま止まり、ログは `dc-load says our IP is 192.168.10.2` で終わった。KOS 2.3 のソースから：
`INIT_NET` + dcload-ip だと `arch_init_net_dcload_ip`（kernel/init.c）が **dbgio を止めてから** `net_init(dcload の IP)` を呼び、
それが **DHCP で同じ IP を要求して最大 60 秒待つ**（net_core.c → net_dhcp_request、直結ケーブルには DHCP サーバーが無い）。
続く `fs_dclsocket_init()` は PC の ARP が引けるまで**無限ループ**（fs_dclsocket.c）。その間コンソールも画面も無音。
原因の特定はソースからの推定で、実機では未確認。

対応：`INIT_NET` を使わず、savemgr の `net.c` と同じ手順で手動で上げる（IP を netif に入れてから `net_init(0)` = DHCP 無し）。
各段に時間制限（net_init 30 秒、PC の ARP 8 秒）を付け、**進捗を画面に出す**（bfont、コンソールは KOS が NIC を持った後は使わない）。

画面の見方（上の行 0 は 1 秒ごとの心拍。`up NNs` が進んでいればフリーズではなく待ち）：

1. `dcload type 1  DC 192.168.10.2  PC 192.168.10.x`（dcload から取った IP。0 なら失敗）
2. `netifs registered: 1` → `net_init ... ` → `net_init rc=0 in NNN ms, default dev bba`（STUCK と出たら BBA / リンクの問題）
3. `MAC ...`、`PC MAC ... (NN ms)`（出なければ `PC did not answer ARP`：DC→PC が通っていない）
4. `fs_dclsocket (exit hand-off): up`、`listening on 192.168.10.2:20039; waiting for the PC sender`
5. PC の送信側が繋ぐと `PC connected` → `PUT received: measuring` → 各パスの結果行。終わると 3 秒表示して dcload に戻る。

失敗したときは `FAILED: <理由>` を出して 20 秒後に戻る。

## 2 回目の実機（p1・p2 は動き、p3 = 64 KiB で止まった）と対応

- **結果が残らなかった**：DC は各パスの終わりに結果の LINE を送っていたが、PC の送信側は最後にまとめて書いていたため、p3 の失敗で p1・p2 の数字も捨てられた。今は LINE が届くたびに `host\g2net_NNN.txt` へ追記・flush し、ログにも出す（`pc_partial=1` のままなら途中で止まった記録）。
- **`SO_RCVBUF` は接続後に変えても窓が変わらない**（KOS の `net_tcp.c`：setsockopt はバッファを realloc するだけで、広告する窓 `rcv.wnd` は accept 時の値のまま。`recv` で窓が開いても ACK は送らない）。つまり前回の p1〜p3 はどれも**既定の 8 KiB の窓**で動いていた（p2 の `rcvbuf=65535` は見かけだけ）。窓が閉じるたびに PC は永続タイマーの探査を待つので、64 KiB のフレームは 16 KiB より遅く、30 秒の無通信で PC が切れた可能性が高い（未確認）。
- 対応：受信バッファはリスナーに接続前に設定（`G2N_LISTEN_RCVBUF`、既定 65535。accept した接続がその大きさ・窓を引き継ぐ。8192 と比べるなら `make net CFLAGS を -DG2N_LISTEN_RCVBUF=8192`）。パスごとの変更はやめた。
- パスの順を「16 KiB（検査あり / なし）→ 64 KiB（depth 1 検査あり / なし → depth 2 あり / なし）」に変え、危ないものを最後にした。
- DATA が 12 秒来なければそのパスを `timeout` と報告し、残りを 5 秒の無音まで捨ててから次のパスへ進む（接続が切れたときだけ中止）。`pN_rx_bytes_total`（途中で切れたフレームも含む）、`pN_outstanding`、`pN_drained_bytes` が残る。
- 心拍の行：`| 123s <段> rx=NNNKB fr=NN`（受信バイト数とフレーム数。止まっていれば通信が止まっている）。PC 側の受信タイムアウトは 90 秒に延ばした。
