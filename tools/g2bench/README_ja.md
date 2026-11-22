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
4. 大きいブロックで dc-tool-ip が落ちる場合（`error: 10055`）は、上限を下げてビルド：`.uild.ps1 -BbaTools ... -MaxBs 4096`（または `$env:G2BENCH_MAXBS`）。
5. 速度が dc-tool-ip の遅延で決まっているかは、`dc_auto.ps1 -Fast`（`dc-tool-ip -f`）と比べる。

## ホストテスト

`sh tests/run_tests.sh`（gcc と python が要る）。CRC-32/FNV の標準ベクタ、データ生成器、`mkdata.py` との一致、key=value の書き出しと読み取り（コメント・CRLF・不正行・桁あふれ）を確認する。

## ファイル

| ファイル | 内容 |
|---|---|
| `main.c` | KOS 側の測定本体 |
| `g2b_core.c/h` | データ生成器、key=value の書き出し・読み取り、速度計算（ホストでテスト） |
| `mkdata.py` | 32 MiB のデータを作る（`g2b_gen_block` と同じ並び） |
| `Makefile` / `build.ps1` / `profiles/g2bench.ps1` | ビルドと dc_auto のプリセット |
| `tests/` | ホストテスト |

## 初回の実機結果と原因の読み取り（2026-11-22）

4 KiB で 38 KB/s（約 105 ms / 呼び出し、最大 165 ms）。16 KiB で dc-tool-ip が `error: 10055`（WSAENOBUFS）を 2 回出して終了。
`H:\DC	ools\dc-bba-toolsuild\dcload-ip` のソースから：

- dcload の `read` は 1 回ごとに dc-tool の `dc_read`（`syscalls.c`）を呼び、これは **ファイルの読み出し + 「PC → DC のバイナリ転送」`send_data`**（アップロードと同じ経路）で返す：`CMD_LOADBIN` の往復 → 1440 B の `CMD_PARTBIN` を連続送信（10 パケットごとに 1800 µs 待つ）→ **（`-f` 無しだと）固定 25 ms の待ち**（`PACKET_TIMEOUT/10`）→ `CMD_DONEBIN` の往復（欠落があれば再送と再度の `DONEBIN`）→ `CMD_RETVAL`。つまり 1 呼び出しあたり固定費が「往復 3 回 + 25 ms」あり、ブロックを大きくすれば薄まる（ただし 1 呼び出しは 1 本の同期処理）。
- `send_cmd` は送信エラーを（`WSAEWOULDBLOCK` 以外）致命とみなして `return -1` する（`dc-io.h`）。`dc_read` は `send_data` の失敗を無視して `CMD_RETVAL` も送ろうとし、それも失敗すると `CatchError` で dc-tool 全体が終了する。2 回続く `10055` はこの流れ。
- 25 ms + 往復では 105 ms に足りない分（約 80 ms）はソースだけでは説明できない。DC 側の取りこぼし→再送、Windows の NIC キューなどを疑うが未確認。1 / 4 / 16 KiB と `raw_4k` の呼び出しあたり時間を比べて、固定費と容量比例の分を分ける。
