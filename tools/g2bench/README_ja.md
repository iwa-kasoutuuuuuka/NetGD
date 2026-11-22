# g2bench：`/pc/` の fread 速度・SH-4 の CRC/FNV 速度・スレッド重ね合わせの測定

`docs/research/g2-perf-plan.md` の **M0**。**Dreamcast + BBA + dcload-ip だけ**で動く（NetGD 基板も GDEMU も不要）。
結果は `host\g2bench_<NNN>.txt` に `key=value` で保存される（`<NNN>` は未使用の最小の 001〜）。

## 安全

- **GD-ROM ドライブには一切触らない**：`KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` で cdrom / fs_iso9660 を初期化せず、G1・ATA・cdrom の呼び出しは無い（gdprobe と同じ流儀）。純正 GDEMU 入りの Dreamcast でそのまま使える。
- 書くのは PC 側の `host\g2bench_NNN.txt` だけ。SD・VMU・フラッシュには書かない。

## 測るもの

| 段 | キー（接頭辞） | 内容 |
|---|---|---|
| A | `read_4k_*` `read_16k_*` `read_64k_*` | `/pc/` の `fread` を 4 / 16 / 64 KiB ずつ、最大 32 MiB。`_kbps`（KiB/s、**fread の中の時間だけ**で割る）、`_us`、`_calls`、`_max_call_us`（1 回の最長）、`_wall_us`（CRC 検証を含む全体）、`_status`（`ok` / `short_read` / `time_limit`＝1 本 120 秒で打ち切り）。64 KiB の回は CRC-32 を検証（`read_64k_crc_ok`、時間には含めない） |
| B | `cpu_crc32_64k_*` `cpu_fnv1a_64k_*` `cpu_both_64k_*` | 64 KiB のバッファ（キャッシュ有り）に対する SH-4 の CRC-32 / FNV-1a / 両方の速度。**`tools/netgd-recv/src/crc32.c` をそのままビルドして使う**。各 1.5 秒。`selfcheck_ok` = 標準テストベクタ |
| C | `ov_cpu_only_*` `ov_read_only_*` `ov_serial_*` `ov_overlap_*` | 8 MiB（128 ブロック × 64 KiB）で、CPU（CRC+FNV）のみ / fread のみ / 直列（fread→CRC+FNV）/ 重ね合わせ（**別スレッドが fread、メインスレッドが CRC+FNV**、3 面のリング）。`overlap_status`：`ok`（別スレッドからの `/pc/` が動いた）/ `timeout`（60 秒進まず）/ `thd_create_failed` / `thread_test_running`（結果ファイルがこの値のまま = 別スレッド中に固まった）。`ov_serial_crc_ok` `ov_overlap_crc_ok` = データの正しさ |

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
3. 所要時間：`/pc/` の速度次第。A は 3 本 × 最大 120 秒、B は約 5 秒、C は 8 MiB × 3 回。BBA 経由で 0.3〜1 MB/s なら全体で 3〜8 分、最悪でも約 10 分。

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
