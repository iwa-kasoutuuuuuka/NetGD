# SAM3U2E のフラッシュの使い道と削減案

2026-11-20、Sonnet。**調査のみ。`mcu/` のファイルは変更していない**（実験はスクラッチパッドの git worktree で行い、終了後に削除した）。LOG・PLAN も未変更（要約は呼び出し側が LOG に書き写すこと）。
実機では何も確かめていない。測ったのはビルド成果物のサイズだけで、速度・タイミングは測っていない。

測定の前提：コミット `aa688ba2` 時点のソース、`DEFMT_LOG=off`、`llvm-size`・`llvm-nm -C -S --size-sort`・`llvm-objcopy -O binary`。
別のエージェントが `mcu/netgd/src` を編集中（LFN 対応）なので、数字は今後ずれる。
「イメージ」は `llvm-objcopy -O binary` の大きさ（512 B 切り上げ、CRC 4 B 込み）。アプリ領域は 122,880 B（0x82000〜0xA0000）。

## 1. 現状の内訳

| ビルド（`resident`、`DEFMT_LOG=off`） | イメージ | 残り（122,880 − イメージ） |
|---|---:|---:|
| 2E・v5.5（Cyclone II を埋め込み） `sam3u2e` | 118,272 | 4,608 |
| 同 ＋ `sd-fast` | 121,344 | **1,536** |
| 2E・6.0 `board-v60`（ビットストリームなし） | 64,512 | 58,368 |
| 同 ＋ `sd-fast` | 67,584 | 55,296 |
| （参考）非 `resident` の v60 / 2E | 66,372 / 120,000 | — |

- 2E・v5.5 には Cyclone II のビットストリーム `BITSTREAM_LZMA` が **49,629 B**（元 73,821 B）入っている（`build.rs` の `RBF_V55`）。コードだけなら `sd-fast` 込みで約 71.7 KB。
  v60 は `.rbf` が無いのでビットストリームもローダーも入らない。差の 約 4.1 KB（R66 の測定は 3,932 B）が「ローダー＋LZMA＋`psload`」。
- R75 の数字（2E `resident,sd-fast` 121,108、v60 既定 66,220）は再現した（121,104、非 `resident` の v60 が 66,216）。
  `resident` の v60 が 2.2 KB 小さいのは常駐部向けのクロック初期化の違い。
- CI が出荷するもの（`.github/workflows/build.yaml:39-63`）：4E は `cargo build --release`（`.cargo/config.toml` の `[env] DEFMT_LOG = "info"`）、
  v60 の `resident` だけ `DEFMT_LOG=off`。**2E の v5.5 は CI に無い**。2E で `DEFMT_LOG=off` を付け忘れると、info では 36 KB 溢れてリンクが失敗する（確認済み）。
- 関数別（`sd-fast`、v60、`llvm-nm`、LTO で inline されるので目安）：`netgd::ide` 約 19.6 KB（`sweep` の closure 1 つで 14.9 KB。ATAPI/ATA コマンドの本体）、
  `netgd::gdi` 13.3 KB、`embassy_executor`（タスクの poll。ユーザーの async 本体が inline されている）7.7 KB、`embedded_sdmmc` 6.7 KB、`netgd::sdcard` 3.3 KB、
  `gamecfg` 3.1 KB、`embassy_atsam3` 2.7 KB、`compiler_builtins`（memcpy/memmove/memset）2.1 KB、`netgd::cdda` 1.2 KB、`discimg` 0.8 KB、`netgd::fat` 0.6 KB。
  `.rodata` は全体で 1.1 KB しかなく、大きな表は無い。`core::fmt` は含まれない（`DEFMT_LOG=off` なら format 機構はもう無い）。
- 重複した CRC 表は **無い**：アプリには表が無く、CRC16 は `discimg/src/atapi.rs:295-306` のビット演算 1 か所だけ。CRC-32 は常駐部（`bootcore`）側。
- R74 の「死んだコードの塊」（`gdi.rs` の `read_ip_header`、`read_at_lba`、`Track::locate` ほか）は **フラッシュを使っていない**：
  `llvm-nm` に現れない（LTO と `--gc-sections` で消えている）。消して得るのは 2E では 0 B。defmt ありの 4E のビルドでは意味があるが、4E は容量に困っていない。

## 2. 削減案の測定

基準は上の表の 4 ビルド。記号：e2＝2E・v5.5、e2f＝同 `sd-fast`、v60、v60f＝同 `sd-fast`。「Δ」はイメージ（または text）の増減。

| # | 案 | 2E v5.5 Δ | v60 Δ | リスク | 手間 | 備考 |
|---|---|---:|---:|---|---|---|
| 1 | `main.rs` の `list_root_dir(dev)` の呼び出しを消す | e2 −2,560 / e2f −2,560 | v60 −2,560 / v60f −2,048 | ほぼ無し。ログ用のルート一覧のためだけに `DEFMT_LOG=off` でも起動時に毎回ボリュームを開いて 2 階層走査している。消せば起動も速くなる。`#[cfg(debug)]` か feature で残せる | 小（1 行） | 呼び出しは `main.rs:174`。info ビルド（4E）は 133 KB に縮む（161 KB → 133 KB。`Debug2Format` の `core::fmt` が減る） |
| 2 | `opt-level = "z"`（`s` から） | e2 −7,680 / e2f −8,192 | v60 −7,168 / v60f −7,680 | **速度が落ちる**（未測定）。IDE のコマンド応答（ホストの BSY の我慢に限りがある）、`push_buf_to_fifo`、CDDA の tick、LZMA 展開（FPGA の立ち上がり時間）が対象。実機でベンチが要る | 小（`Cargo.toml` 1 行）＋実機 | `netgd` クレートだけ `z`、他は `s` でも −7,164（e2f）で、ほぼ同じ。`[profile.release.package."*"]` だけ `z` は逆効果（+540）。`lzma`・`embassy-atsam3`・`psload` を `s` に戻すと +536 B で済む（tick/LZMA を守る妥協案） |
| 3 | `.trim()` を ASCII 専用の `#[inline(never)]` 関数に置き換え（`gamecfg`・`gdi.rs`） | −508 | −576 | Unicode の空白を削らなくなる（INI では実害なし）。`gamecfg` のホストテストで確認 | 小 | 単に `str::trim_ascii()` にすると **逆に +524**（inline で複製される）。共有の非 inline 関数にしたときだけ効く |
| 4 | LLVM の machine outliner（`-C llvm-args=-enable-machine-outliner=always`） | −1,020（`s` のとき） | −1,032 | 呼び出し/復帰が増えて遅くなる。`z` と併用すると 0（効果なし） | 小 | 2 と排他的。採らない |
| 5 | リンカーの `--icf=all` | +28（`s`）/ −488（`z`） | −260（`s`） | 小 | 小 | 誤差程度。採らない |
| 6 | `panic-probe` を `loop{}` のハンドラーに置き換え | −12 | −44 | パニック時に defmt に出なくなる | 小 | `DEFMT_LOG=off` では既にスタブ（`__defmt_default_panic` 10 B）。**やる価値なし** |
| 7 | `sd-fast` の内訳 | `sd-multiblock` +1,024、`sd-highspeed` +2,048、`fast-complete` −8、合計 **+3,064** | 同 | 速さは機能側の問題 | — | `fast-complete` はサイズ面では無料。`sd-highspeed` が一番高い（R73/R75 の検証コード込み） |
| 8 | `DEFMT_LOG` の水準（`error`/`warn`/`info`） | 出荷は `off` のまま。v60f（resident）で error +796、warn +7,304、info +14,816。4E 実測（元のソース）では off→info が +41,260 | 同左 | — | — | 2E 出荷は `off` 固定が必要。v60 は容量に余裕があるので、開発用に `warn`（+7.3 KB）までなら載る |
| 9 | `embedded_sdmmc` を自前の FAT（`fat.rs`）で置き換える | **推定** −4〜−7 KB（未測定） | 同左 | 中〜大。ディレクトリ一覧・LFN・サブディレクトリ・ファイル読み（ini、gdi、cdi、遅い経路）を作り直す。動作が変わりうる | 大（数日）＋ホストテスト | 根拠：`embedded_sdmmc` の記号だけで 6.7 KB（`z` では 7.2 KB）、`gdi` が 13.3 KB（`z` で 10.1 KB）。自前の `fat.rs` は 0.6 KB で、すでに FAT のチェーン・SFN 検索・パーティション開始を持つ。足りないのはディレクトリ列挙、LFN の組み立て（約 250 B）、ファイル読み。**LFN 対応が今進行中（R77）なので、その結果を見てから決める** |
| 10 | `lzma` のデコーダー | −（小さい） | 同左 | — | — | ローダー込みで 約 4.1 KB（`decode_len` 786 B ほか）。削る余地は小さい |
| 11 | nightly の `-Zbuild-std` ＋ `panic_immediate_abort` ＋ `-Zlocation-detail=none` | 未測定 | 未測定 | stable 前提の CI が変わる | 中 | このマシンの toolchain は stable だけで測れなかった。通常は数 KB の効果が見込まれるが `.rodata` が 1.1 KB しか無いので小さい見込み（推測） |
| 12 | 既に有効な設定 | — | — | — | — | `lto = "fat"`、`codegen-units = 1`、`panic = abort`（ターゲット標準）、`-C no-vectorize-loops`。debug = 2 はフラッシュに影響しない |

### 組み合わせ（イメージ、`DEFMT_LOG=off`、`resident`）

| 構成 | e2（v5.5・2E） | e2f | v60 | v60f |
|---|---:|---:|---:|---:|
| 現状（`s`） | 118,272 | 121,344 | 64,512 | 67,584 |
| 1 のみ | 115,712 | 118,784 | 61,952 | 65,536 |
| 2 のみ（`z`） | 110,592 | 113,152 | 57,344 | 59,904 |
| 1＋2 | 108,544 | 111,104 | 55,296 | 57,856 |
| 1＋2＋3 | 108,032 | 110,592 | 54,784 | 57,344 |
| 1＋2＋3 での 2E の残り（122,880 −） | 14,848 | **12,288** | 68,096 | 65,536 |

## 3. 6.0 のビットストリームの余地（R66 の案ごと）

埋め込みの上限 X ＝ 122,880 − （v60 のイメージ ＋ ローダーと LZMA 3,932 B）。伸びしろ 8 KB を残す場合は −8,192（R66 と同じ前提）。
`z` ではローダーも小さくなるので実際はこれより数百 B 多い（未測定）。

| 構成 | X の上限（`sd-fast` なし） | 同（伸びしろ 8 KB） | X の上限（`sd-fast`） | 同（伸びしろ 8 KB） |
|---|---:|---:|---:|---:|
| 現状（`s`） | 54,436 | 46,244 | 51,364 | 43,172 |
| 1 のみ | 56,996 | 48,804 | 53,412 | 45,220 |
| 1＋2（`z`） | 63,652 | 55,460 | 61,092 | 52,900 |
| 1＋2＋3 | 64,164 | 55,972 | 61,604 | 53,412 |

（R66 の「53,532 / 45,340」は常駐部ありで `sd-fast` なし・古いソースの値で、現状の行と 2 KB の差があるのは 1 の前の最近の変更による。）

- **案 a（埋め込み）**：見積もり 55〜75 KB のうち、`z` と 1 を入れた `sd-fast` 込みで **約 61 KB（伸びしろ 8 KB 込みで 53 KB）まで**入る。
  下側の 55〜61 KB なら入る（伸びしろは足りないか無い）が、上側の 61〜75 KB・模擬の 61〜84 KB は入らない。`sd-fast` を外せば +3 KB。現状のままでは 51 KB（伸びしろ込み 43 KB）で、見積もりのほぼ全域が入らない。
  **削減案で a が「常に成り立つ」ようにはならない**。Supra で測った X が 53 KB 以下なら a が現実的になる、というのが変化。
- **案 b（SD の `NETGD.BIT`）**：アプリは v60f で 57〜68 KB なので、フラッシュの空きは 55〜65 KB。読み込みコードは LZMA が要らない分 R66 の見込み（`kind = 2` ヘッダ検査＋CRC＋`psload`）で 1〜2 KB 程度。
  削減案が無くても余裕があり、削減案を入れれば後続の機能（LFN、BBA 書き込み、G2）の伸びしろが 10 KB 前後増える。**R66 の結論（SD 優先）は変わらない**。
- **案 c1（SD 優先、埋め込みは予備）**：a が入るときだけ意味がある。X が 53 KB 以下で初めて両立できる。
- **案 c2（最小のビットストリームだけ埋め込む）**：X が 10〜30 KB なら現状でも入る（空の設計で 10.4 KB）。サイズ面の制約はない。
- **案 d（LZMA の辞書 8 KiB）**：約 −2 %（1.2〜1.4 KB）。削減案が先に効くので、a が僅差のときの最後の一押し。

## 4. 2E・v5.5（Cyclone II を埋め込んだ 2E）について

現状の残りは `sd-fast` ありで 1,536 B。LFN 対応など次のコード追加で簡単に溢れる。1＋2 で 約 11.8 KB、3 を足して 12.3 KB の余地が出る。
2E に v5.5 を載せるビルドは CI に無い（`build.yaml` は v60 だけ `resident` で 2E 相当）ので、**溢れてもリンクが失敗するまで気づかない**。
CI に `sam3u2e,resident,sd-fast`（`DEFMT_LOG=off`）を追加するのを勧める。

## 5. 勧め

1. **すぐ入れてよい（リスクなし）**：案 1（`list_root_dir` の呼び出し削除、−2.5 KB）。案 3（`trim` の共有化、−0.5 KB）は任意。
2. **`opt-level = "z"` は最大の効果（−7.2〜8.2 KB）だが、実機のベンチが先**。まず `netgd` だけ `z`、`lzma`・`embassy-atsam3`・`psload` は `s` に戻す形（+0.5 KB）。
   見る点：IDE のコマンド応答（BSY の我慢）、CD_READ の DMA（`stream_from_sd`）、CDDA の `on_tick`、FPGA の立ち上がり時間（LZMA）、Cyclone の PS 書き込み。悪ければ `s` に戻す（1 行）。
3. **`embedded_sdmmc` の置き換えは今は着手しない**。LFN 対応（R77）が `fat.rs` を伸ばすなら、その延長でディレクトリ列挙とファイル読みまで作れば −4〜−7 KB が見込める。LFN 担当の結果を見てから判断（推定のみ、未測定）。
4. 採らない：outliner（z と排他的、遅くなる）、`icf`、`panic-probe` の置き換え、`trim_ascii` への単純置換（逆効果）。
5. 2E の出荷は `DEFMT_LOG=off` 固定（info は 36 KB 溢れる）。v60 の開発ビルドは `warn`（+7.3 KB）まで載る。
6. 6.0 の方針（SD 優先）は、サイズの削減では変わらない。Supra で X が分かるまでの判断（LOG の判断表）と整合する。X ≤ 53 KB なら 1＋2 を入れて埋め込みも選べる、と記録すること。

## 6. 再現

```
# スクラッチの worktree（mcu/netgd と path 依存が相対パスで揃う）から。--target-dir は使わない
cd mcu/netgd
DEFMT_LOG=off cargo build --release --features sam3u2e,resident,sd-fast                              # e2f
DEFMT_LOG=off cargo build --release --no-default-features --features board-v60,resident,sd-fast     # v60f
DEFMT_LOG=off CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --release ...                            # 案 2
llvm-objcopy -O binary target/thumbv7m-none-eabi/release/netgd out.bin && stat -c %s out.bin         # イメージ
llvm-nm -C -S --size-sort -td target/thumbv7m-none-eabi/release/netgd                                # 関数別
```

信頼度：サイズは確認済み（ビルド成果物）。削減案 9・11 と `z` のときのローダーの縮みは推測／未測定。速度への影響は未測定。Opus による検証：なし。
