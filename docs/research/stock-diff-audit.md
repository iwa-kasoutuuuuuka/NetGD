# 純正 GDEMU 6.0 の全記録と NetGD の予測の総点検、2E `release-z` の大きさの内訳

調べた日：2026-11-22。基準：main `6ef6dc87`。D14 だけはブランチ `feat/form2-sectors`（`b7b3b578`、main + D14）の予測で見た。
ファームは変えていない（文書だけ）。実機では何も走らせていない（記録は既存のもの）。

## 1. 方法

- 予測：`common/discimg/examples/expect.rs`（`feat/form2-sectors` でビルドした実行ファイル）を `tools/netgd-expect/expect_sd.py` から呼ぶ。
  記録ごとにヘッダの `# config` 行から設定を写す：`images`・`return_to`・`cdread_flags`・`ip_fad`・`tp`（`--tp-batches`/`--tp-sectors`）・`seek`（`--seek-n`、`0` なら `--no-seek`）・`throughput=0`（`--no-throughput`）。
- イメージ：`H:\NetGD_probe_sd` の `01`〜`07` と `H:\NetGD_probe_sd_v3` の `08`〜`10` を、1 つのフォルダにジャンクションで並べて使った（記録 06 の `08` は `v3` のマニフェストと 1,050 件一致するので、同じ中身）。
- 2 通り：**既定**（main の既定ビルド）と **機能あり**（`--raw-sectors --form2-sectors`）。
- 比較：`python tools/gdprobe/compare.py --only-common 予測 記録`。片側だけの行（予測しない `img=0` の電源投入直後の試験、`IDENTIFY_PACKET` など）は数えない。
- 記録 02（`switch=0`、`img=0` だけ）は、電源投入時のディスクをフォルダ `02` の `disc.cdi`（dcload-ip）として、`--img 0 --no-ua` で予測した。
- 記録 01（`abort`、G1 バスの保護が閉じていて最初の TUR がタイムアウト。R117）は比べられないので外した。
- データの照合：全記録に `compare.py --manifest` を 2 つのマニフェストで実行し、照合できた HASH は**全件一致、不一致 0**
  （例：04 は 1,056 件、05 は 2,098 件、13 は 547 + 273 件）。

## 2. 記録ごとの差の数

| 記録 | 内容 | 既定 | 機能あり | 機能ありで残る差 |
|---|---|---:|---:|---|
| 01 abort | G1 バスの保護で中断 | — | — | 比較なし |
| 02 | `img=0`（dcload-ip の CDI） | 2 | 1 | D15 |
| 03 img3 | GDI 03 | 2 | 1 | D15 |
| 04 img3-7 | 03〜07 | 20 | 17 | D15 × 5、D11（06）× 12 |
| 05 cdi-tp | CDI 04・05、`0x20` | 8 | 2 | D15 × 2 |
| 06 gdi-variants | 03・06・08・09 | 30 | 28 | D15 × 4、D11（06・09）× 24 |
| 07 img9-recheck | 09 | 13 | 13 | D15、D11（09）× 12 |
| 08 identify | 03（IDENTIFY あり） | 2 | 1 | D15 |
| 09 missing-folder | 50（無い番号 → 01）・03 | 4 | 2 | D15 × 2 |
| 10 ini-600-300 | 03・50 | 4 | 2 | D15 × 2 |
| 11 ini-no-time-keys | 03・04 | 6 | 2 | D15 × 2 |
| 12 ini-restored | 03（読みなし） | 1 | 1 | D15 |
| 13 r137-abg | 04・10・03・07 | 9 | 4 | D15 × 4 |
| 14 audio-flags | 03 の音声（FAD 906） | 1 | 1 | D15 |
| **計** | | **102** | **75** | |

## 3. 残っている差の一覧と分類

| # | 差（`compare.py` の行） | 出る記録 | 分類 | 内容・場所・案 |
|---|---|---|---|---|
| 1 | `GDEMU_VERSION: data differs (+2, +3, +5)` | 全記録・全 `img`（計 27 行） | **既知・受け入れ（D15）** | 純正 `00 00 05 02 00 05 20 05`（ブートローダー 2.5.0、ファーム 5.20.5）、NetGD `00 00 00 00 00 00 20 05`（ブートローダー 0.0.0、互換版 5.20.0）。R118 で「NetGD 自身の版を返す」と決定。`ide.rs`（`52 00` の応答、`resident::bootloader_version_word`）。変更しない |
| 2 | `CDREAD_FLAGS flags=0x10: bytes 2048 vs 2352` | 02〜06・08〜11・13（計 19 行） | **機能で対応済み（D10）** | feature `raw-sectors`（main、既定オフ）で消える。実機未確認 |
| 3 | `SEEK_01: bytes 2048 vs 2324` と `HASH ... ssz 2324 crc=d0912a98` | 05（img 4・5）、11、13（計 8 行） | **機能で対応済み（D14）** | feature `form2-sectors`（ブランチ `feat/form2-sectors`、未取り込み）で消える。実機未確認 |
| 4 | `img=6`・`img=9` の 12 行（`SW_POLL#2` 6/28/00 vs 2/3a/00、`SW_RESULT` ok vs timeout、`WARMUP`、`TUR`、`REQ_STAT`、`GET_TOC_*`、`REQ_SES_*`、`GET_SCD_*`） | 04（06）、06（06・09）、07（09）（計 48 行） | **未対応（D11、方針の問題）** | 下の 3.1 |
| 5 | 予測ツールの誤り | — | **見つからず** | 機能ありで残る差は 1・4 だけで、どちらも原因が分かっている |

### 3.1 D11：純正が受け付けない GDI の形

- 純正は、低密度領域のトラックがちょうど 2 本（トラック 1 = データ LBA 0、トラック 2 = 音声）で、高密度領域がトラック 3（LBA 45000）から始まる GDI だけをマウントする（R121・R122。03・08 は受け付け、06（低密度に音声 3 本）・09（同 2 本 + データ）は拒否）。
  拒否すると、切替の窓のあとも 2/3A/00（ディスクなし）のままで、gdprobe は 15 秒でタイムアウトする。
- NetGD はどんな形でもマウントする（`discimg::atapi::gdi_boot_area_lba` は「3 番以降の最初のトラック、無ければ 45000」）。
- 場所：`mcu/netgd/src/gdi.rs` の GDI の読み込み（`gdi_boot_area_lba` を呼ぶところ、1241 行付近）、予測側は `common/discimg/examples/expect.rs` の `load_disc`。
- 案（入れるなら）：`discimg` に純粋関数 `gdi_stock_layout(tracks) -> bool`（トラック 3 が LBA 45000 で始まり、トラック 1 がデータ・トラック 2 が音声）を足し、
  偽ならマウントせず「ディスクなし」（D12 の空フォルダと同じ扱い）にする。`expect.rs` も同じ関数で `--empty-folder` 相当の予測にする。ホストテストは 03・06・08・09 の形。
- 勧め：**今のまま（受け付ける）**。市販の GD-ROM から作った GDI はこの形を外れない（R121）ので、拒むことで得るものは無く、自作の GDI が動かなくなるだけ。
  純正と完全に同じ動きが要る場面（純正の挙動の再現試験など）が出たら、既定オフの feature で入れる。

### 3.2 この点検で見ていないもの（予測ツールの対象外）

- 記録 03〜14 の `img=0`（電源投入直後の試験）。`expect_sd.py` は `img=0` を予測しない（記録 02 だけ別に予測し、D15・D10 以外は一致）。
- `IDENTIFY_PACKET`（記録 08）、時間（`us`・`t_ms`）、`SW_POLL` の回数。
- **`cdbst=`・`secnr=`（記録 13・14）**：予測ツールはこの 2 つの欄を出さないので、`compare.py` は比べない（両方にあるときだけ比べる）。
  R144 の a（CDB フェーズの DRDY、R147 で訂正済み）・b（切替中の SECNR）は、この点検では確かめていない。
  確かめるなら `expect.rs` が `secnr=`（`discimg::atapi::swap_secnr` と `rest_secnr` 相当）と `cdbst=`（直前の完了の status から）を書くようにする（予測ツールの追加作業）。

## 4. 2E `release-z`（`resident,sd-fast`、`DEFMT_LOG=off`）の大きさ

ビルド：`DEFMT_LOG=off cargo build --profile release-z --features sam3u2e,resident,sd-fast`（main `6ef6dc87` と `c8d2cb02`、別々の worktree）。

| | `c8d2cb02` | main `6ef6dc87` | 差 |
|---|---:|---:|---:|
| text（`arm-eabi-size`） | 117,528 | 118,544 | +1,016 |
| data | 160 | 160 | 0 |
| うち `.text`（コード） | 64,620 | 65,668 | +1,048 |
| うち `.flash1`（FPGA のビットストリーム、LZMA） | 51,372 | 51,372 | 0 |
| 上限 122,368 までの残り（text + data） | 4,680 | **3,664** | −1,016 |

- 注意：同じソースでも worktree が違うと ±20 B ほど変わることがある（D14 の作業で `f0dccd36` を別の worktree でビルドすると 118,524 だった）。
- このビルドが埋め込むのは **v5.5 の Cyclone II のビットストリーム**（`fpga/Quartus/output_files/OpenGDEMU.rbf`、74,685 B → 51,372 B）。
  GDEMU 6.0（AG16K）のビットストリームはまだ無く、その大きさで残りは全く変わる（`flash-budget-2e.md` 3 節）。

### 4.1 大きい記号 20 個（main）

| 順 | 大きさ（B） | 記号 |
|---:|---:|---|
| 1 | 51,372 | `netgd::BITSTREAM_LZMA`（`.flash1`） |
| 2 | 8,956 | `netgd::ide::handle_packet::{closure}` |
| 3 | 4,332 | `main_task` のタスク本体（`TaskStorage::poll`） |
| 4 | 4,236 | `netgd::ide::sweep::{closure}` |
| 5 | 3,764 | `netgd::gdi::load_image` |
| 6 | 2,340 | `netgd::sdcard::init_once::{closure}` |
| 7 | 1,824 | `button_task` のタスク本体 |
| 8 | 1,432 | `compiler_builtins::mem::memmove` |
| 9 | 1,292 | `netgd::fat::lookup` |
| 10 | 1,224 | `netgd::gdi::walk_to` |
| 11 | 1,110 | `netgd::gdi::locate_image` |
| 12 | 904 | `ide_task` のタスク本体 |
| 13 | 852 | `netgd::gdi::step_folder` |
| 14 | 852 | `embedded_sdmmc::VolumeManager::open_volume` |
| 15 | 848 | `embassy_atsam3::init` |
| 16 | 812 | `cdda_task` のタスク本体 |
| 17 | 808 | `netgd::ide::fill_cd_read_buf_from_sd::{closure}` |
| 18 | 798 | `netgd::gdi::folder_lookup` |
| 19 | 792 | `lzma::Decoder::decode_len` |
| 20 | 682 | `netgd::ide::send_data_in::{closure}`（同じ大きさで `compiler_builtins::mem::memcpy`） |

まとまりごと（コード）：`netgd::ide` 18,276、`netgd::gdi` 11,954、タスク本体（`embassy_executor` 名義） 8,304、`embedded_sdmmc` 6,580（ほかに `File`・`Volume` の drop で 450）、
`core::` 3,066（うち文字列処理 約 1,400：`from_utf8` 368、`slice_error_fail_rt` 282、`CharSearcher` 156、`Chars::next` 100 など）、`netgd::sdcard` 2,986、`embassy_atsam3` 2,272、
`compiler_builtins` 2,114（`memmove` + `memcpy`）、`netgd::fat` 2,032、`gamecfg` 1,204、`lzma` 792、`discimg` 688。

### 4.2 `c8d2cb02` からの増え方（+1,016 B）

| 増減 | 記号 | 由来 |
|---:|---|---|
| +798 / −636 | `gdi::folder_lookup`（新）/ `gdi::folder_for_number` | R139：`52 82 n` の探索で読み取りエラーと「フォルダ無し」を分けた（差し引き +162） |
| +176 | `ide::handle_packet` | R139・R148 |
| +144 | `embassy_atsam3::gpio::pioa_wait_irq`（新） | R148：切替の窓の SECNR をタイマーで進める（`ide_task` の待ちを信号かタイマーに） |
| +108 | `ide::fill_cd_read_buf_from_sd` | R148：音声トラックの 2,352 B の経路を既定のビルドに |
| +84 / +80 / +48 / +40 / +30 / +28 / +26 | `window_secnr`・`mounted_disc_format`・`swap_secnr`・`rest_secnr`・`window_elapsed_ms`・`disc_format`・`Instant + Duration` | R148：SECNR とディスク形式をトラックのモードと窓の段階から |
| 残り | `load_image` +40、`step_folder` +26、`fill_cd_read_buf_raw` +24 ほか小さいもの | R148（トラックのモードを詰める）など |

### 4.3 削減の案（コードはまだ変えていない）

`flash-budget-2e.md` 2 節・7 節の案（`opt-level = "z"` は `release-z` で使用済み、`sd-list` は既定で外れている）に加えて、今回の内訳から：

| 案 | 見込み | リスク | 手間 | 備考 |
|---|---:|---|---|---|
| A. `handle_packet` の使用頻度の低いコマンド（`0xE0` のイメージ操作、`52` の副コマンド、REQ_MODE、GET_SCD、REQ_SES など）を、応答をバッファに作るだけの async でない `#[inline(never)]` 関数に分け、`handle_packet` には `send_data_in(...).await` だけを残す | 1〜2 KB（推定、未測定） | 小（動作は同じ。応答のバイトはホストテストのビルダのまま） | 中 | async の状態機械は await をまたぐ値と分岐を抱え込むので、await の前の計算を外に出すほど小さくなる。`ide.rs` はメインの担当 |
| B. `sweep` の中身（4,236 B）の確認：`handle_cd_read` などの async 関数が inline で取り込まれていないか見て、必要なら `#[inline(never)]` | 0.5〜1.5 KB（推定） | 小 | 小 | 呼び出しは 4 か所（`ide.rs` 801・802・835・836）、本体は 1 つ。中身を `objdump` で確かめてから |
| C. 名前の解析を `&str`（UTF-8 の検証・`char` の走査・`split`）から `&[u8]` に置き換える（`gdi::configured_name`、`gamecfg` の `trim`・`split`） | 0.8〜1.2 KB（`core::str` の 約 1.4 KB の大半） | 小（INI・GDI は ASCII。ホストテストあり） | 小〜中 | `flash-budget-2e.md` 案 3（`trim` の共有化 −0.5 KB）を含む。`from_utf8` 368 B と `slice_error_fail_rt` 282 B が消えるかは、全部の `&str` を外せたときだけ |
| D. `embedded_sdmmc` を自前の `fat.rs` に寄せる | 4〜7 KB（`flash-budget-2e.md` 案 9、`embedded_sdmmc` が今 6.6 KB + drop 0.45 KB） | 中〜大 | 大 | 一番大きいが作り直しになる。FAT のコードがすでに 2 つある（`fat.rs` 2,032 B と `embedded_sdmmc`） |
| E. `compiler_builtins` の `memmove`（1,432 B）を小さい実装に | 約 1 KB | 中：CD_READ の詰め直し（`copy_within`）の速さが落ちる。シンボルの置き換えはリンカーとの相性がある | 中 | 速さが要る経路なので勧めない。最後の手段 |
| F. LZMA の辞書を大きく | 8 KiB で −99 B、12 KiB で −200 B、64 KiB で −554 B（`xz --format=raw`、v5.5 の `.rbf` で実測） | 辞書の分だけ RAM が増える（`flash-budget-2e.md` 7.3） | 小 | **効果が小さく、勧めない**。`flash-budget-2e.md` の案 d の「約 −2 %（1.2〜1.4 KB）」は、この `.rbf` では当てはまらない（0.2 %） |
| G. `lc`・`lp`・`pb` の変更 | 逆効果（`lc=3,pb=2` で +305 B、`lc=1` で +132 B） | — | — | 今の `lc=0 lp=0 pb=0` が最小 |

勧め：
1. 次に `ide.rs` を大きく触るときに A と B を一緒に行い、前後で `arm-eabi-nm --size-sort` を比べる（合わせて 1.5〜3 KB の見込み。推定）。
2. C は `ide.rs` に触らずにできるので、先に入れられる（0.8〜1.2 KB の見込み）。
3. D は、6.0 のビットストリーム（AG16K）の大きさが分かり、余裕が足りないと分かってから。
4. E・F・G は採らない。

## 4.4 追記：予測ツールが `cdbst=`・`secnr=` を書くようにした（R144 a・b の点検）

ブランチ `feat/expect-cdbst-secnr`（`docs/stock-audit` から）。変えたのは予測ツール（`common/discimg/examples/expect.rs`・`tools/netgd-expect/expect_sd.py`）とそのテスト・README だけで、ファームは変えていない。

- **`cdbst=`**：直前のコマンドの最終 status が CHECK なら `0x18`、それ以外は `0x58`（R147）。gdprobe が記録しないコマンドも数える：
  `CDREAD_FLAGS` の CHECK のあとの REQ_ERROR、切替のポーリング（TUR と、CHECK なら REQ_ERROR）。区間をまたぐ状態は `expect_sd.py` が渡す（`--prev-check`）。
- **`secnr=`**：main（`rest_secnr`・`window_secnr`）と同じ規則。試験の間は「ディスク形式 ｜ 一時停止」（電源投入直後は待機）、ディスクなしは `0x07`、
  切替の窓は `0x06` → `0x00` → 窓のあと「形式 ｜ 0x01」か `0x07`。`SW_CMD` は切替前のディスクの値（`--prev-secnr`）。
  最初の切替の前（電源投入時のディスク）の値は分からないので書かない（`compare.py` は両方にある欄だけ比べる）。
  `fix/secnr-reset`（`237130e6`）の `reset_secnr` はバスリセットのときの値で、gdprobe の試験では起きないので予測には入れていない。
- **結果**：記録 13・14 の `cdbst=`・`secnr=` は**すべて一致**（記録 13 は予測の `cdbst=` 171 行・`secnr=` 189 行、14 は 23 行・30 行。差の数は増えない）。
  記録 13 は既定 9 → `--raw-sectors` で 6（D15 × 4、D14 × 2。D14 はこのブランチに無い）、14 は 1（D15）のまま。ほかの記録は欄が無いので数は変わらない（2 節と同じ）。
- 比べていることの確認：予測の 2 行の `cdbst`・`secnr` をわざと変えると、`compare.py` がその 2 行を差として出す（8 件になる）。
- テスト：`tools/netgd-expect/test_expect.py` に `test_cdbst_and_secnr_follow_stock` を追加（CHECK の次の `0x18`、`CDREAD_FLAGS` の隠れた REQ_ERROR、窓の `0x06`/`0x00`/`0x81`、
  `SW_CMD` が前のディスクの値、空フォルダの `0x07`、最初の `SW_CMD` に `secnr` が無いこと）。14 件（1 件スキップ）通過、`common/discimg` 全通過。
- 意味：NetGD の main は、ソースの読みでは R144 a（R147 で訂正）・b について純正と同じ。実機の NetGD で gdprobe（`label=netgd`）を取れば、この予測と比べて確定できる。

## 5. 再現

```sh
# 予測（feat/form2-sectors の expect、または main の expect で --form2-sectors を外す）
cd common/discimg && cargo build -q --example expect
python tools/netgd-expect/expect_sd.py <01〜10 を並べたフォルダ> --images 4,5 --return-to 2 \
  --bin common/discimg/target/debug/examples/expect.exe --cdread-flags 0x20 --tp-batches 64 --tp-sectors 16 --seek-n 16 \
  [--raw-sectors --form2-sectors] --out pred05.txt
python tools/gdprobe/compare.py --only-common pred05.txt docs/research/probe-records/stock_20261121_05_cdi-tp.txt

# 大きさ
cd mcu/netgd && DEFMT_LOG=off cargo build --profile release-z --features sam3u2e,resident,sd-fast
arm-eabi-size -A target/thumbv7m-none-eabi/release-z/netgd
arm-eabi-nm -S --size-sort -C target/thumbv7m-none-eabi/release-z/netgd | tail -25
xz --format=raw --lzma1=preset=9e,dict=4KiB,lc=0,lp=0,pb=0 -c fpga/Quartus/output_files/OpenGDEMU.rbf | wc -c
```

## 関連

LOG R117〜R122（記録 01〜07）、R118（D15）、R121・R122（D11）、R123・R138（D10・D14）、R129・R135（INI の時間）、R139、R144・R147・R148。
`docs/research/flash-budget-2e.md`、`docs/research/d10-d14-raw-sectors.md`、`tools/netgd-expect/README_ja.md`。
