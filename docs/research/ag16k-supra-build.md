# AGM Supra 2026.03 で NetGD の AG16K 版を実際にビルドした結果（ビットストリームの実サイズ）

2026-11-21、Fable（ツールチェーンの立ち上げ）。LOG への転記は呼び出し側が行う。Opus による検証：なし。
**実機では何も確かめていない。** 分かったのは「Supra がエラーなくビットストリームを出した」ことと、その大きさ・資源・タイミングの報告だけ。
ピン配置は**仮**（6.0 の配線は未判明）なので、ここで作った `.bin` / `.rbf` を基板に流してはいけない。

## 0. 要約

| 項目 | 結果 | 信頼度 |
|------|------|--------|
| ライセンス | 同梱の `license/license.txt`（ノードロック表記、期限 2036-03-31）のまま、**拒否もメッセージも無く**最後まで動いた。回避策は何も使っていない | 確認済み |
| フロー | `af_setup` → `af_map`（Yosys 0.61 + `af_map.tcl`）→ `af_run --mode NATIVE` で配置配線とビット生成まで通る。**ただし inout の扱いに後処理が要る**（§4-1） | 確認済み |
| FIFO → ブロック RAM | **5 個とも M9K（`alta_bram9k` 14 個）に載った**。R39 の懸念（LUT への展開）は起きない | 確認済み（`map.log`・配置配線後のネットリスト） |
| 資源 | ロジック 2,291 / 16,624（13 %）、LUT 1,980、FF 1,089、M9K 14 / 56、PLL 1 / 4、ピン 66、グローバル信号 3 / 20 | 確認済み（Supra の報告） |
| タイミング（仮ピン） | 48 MHz 系 Fmax 90〜98 MHz、11.2896 MHz 系 138〜175 MHz。`DC_DATA` への 20 ns 制約は余裕 +2.1〜+3.0 ns | 確認済み（Supra の報告。仮ピンでの値） |
| **非圧縮 `.bin` / `.rbf`** | **552,352 B**（設計に依らず固定） | 確認済み |
| **非圧縮 `.rbf` → LZMA（リポジトリの設定）** | **37,727 B**（シードとクロックピンを変えた 3 回で 37,727〜38,609 B。`.bin` なら 37,368〜38,277 B） | 確認済み（`common/lzma` のデコーダで往復も確認） |
| AGM 圧縮（LZW、Supra の既定）の `.bin` | 255,887 B。これを LZMA にかけても 71,381 B | 確認済み |
| R32 の見積もり（55〜75 KB）との比較 | **実測は約 38 KB で、見積もりの下限より 17 KB 小さい** | — |
| R81 のしきい値（53 KB）との比較 | **約 15 KB 下回る。** `opt-level="z"` などの削減を入れない今のコード（`sd-fast` あり・伸びしろ 8 KB 込みの上限 43,172 B）でも入る計算 | サイズは確認済み。フラッシュ側の数字は R81 時点のもの |

→ **「埋め込み（R66 の案 a / c1）はサイズの面で成り立つ」**。SD 方式でないと入らない、という前提は外れた。
残る論点はサイズではなく、非圧縮 4.42 Mbit を PS で流す時間（`ag16k-config.md` §4-3）と、実ピンでの再測定。

## 1. 環境と版

| もの | 版・場所 |
|------|----------|
| OS | WSL2 Ubuntu 24.04.5 LTS（カーネル 6.18.58.1-microsoft-standard-WSL2）、Python 3.12.3、xz 5.4.5 |
| Supra | `~/supra-linux/x/Supra-2026.03.b0/`。`af --version` = `2026.03.b0`、ログの表記は `2026.03.b0(a052d0a8)` |
| `bin/af` の SHA-256 | `d107b2727c94e2304b2a82c0d80a612c6f81e38cd768b7c79965d3f03e09da55`（Linux 版。R92 の値は Windows 版） |
| `map/bin/yosys` | Yosys 0.61（git sha1 c18d47a44）、SHA-256 `b5ec6e388996b69b6448024154f17dc38424d0ac62a17e94be4bd5ff2e5ff7b2` |
| デバイス | `AG16KDDF256`（Supra は Quartus 側の対応品を `EP4CE15F23C8`、ファミリを Cyclone IV E として扱う。`OpenGDEMU.qsf` に出る） |
| 入力のコア | `fpga/amaranth/out/OpenGDEMUCore_ag16k.v`（`main.py --target ag16k` で再生成。コミット済みのものと一致、`git status` は clean。LF 版の SHA-256 `906ce6d0…4296cb`、368,786 B） |
| 作業場所 | `~/supra-linux/work/`（WSL 内。リポジトリには Supra のファイルも生成物も置いていない） |

### 安全上の扱い（指示どおり）

- Supra の実行ファイル（`af`・`yosys`）は**すべて `unshare -rn` の中**で実行した（ラッパ `in/ns.sh`）。中のインターフェースは `lo`（DOWN）だけ。
- 実行したのは `af_cmd`（`--help`・`--version`・`-x help`・`generate_pll`）、`af_setup`、`af_map`、`af_run` だけ。
  `Supra`（GUI）、`Downloader`、`urjtag`、`openocd`、`af_program`、`af_downloader`、`af_flashloader`、ライセンスの request / import / export / append は実行していない。`sudo` も使っていない。
- **通信の試みについて**：`strace` が入っていないのでシステムコールは見ていない。ログ（`run.out`・`alta_logs/*`・`map.log`）に、ソケット・接続・名前解決・ライセンスに関するエラーや警告は 1 行も無かった。
  `af` は `socket`・`connect`・`getaddrinfo`・`gethostbyname_r`・`ioctl` をインポートしている（`nm -D`。Tcl のソケット機能と、ノードロックの判定に使うものと思われる。推測）。
  ネットワークの無い名前空間で最後まで動いたので、少なくとも**通信ができないと止まる作りではない**。
- Supra のツリーへの書き込み：実行後に `find -newermt` で見て、**変更されたファイルは無し**。ツリーの外では Yosys が `~/.local/state/yosys/` を作った（履歴用）。それ以外に `$HOME` に増えたものは無い。

## 2. フローの実際（R69 / R92 の更新）

`g1-ag16k-port.md` §2-1 の「`synth_agm -vqm` → `af`」は古い版の話（R92 で訂正済み）。2026.03 では次のとおり動いた。

```
af_setup --design OpenGDEMU --device AG16KDDF256 --verilog OpenGDEMU.v Audio_PLL.v OpenGDEMUCore_ag16k.v
    → カレントに af_map.tcl / af_run.tcl / af_batch.tcl / *.qsf / 空の *.asf を作る（etc/ のひな形を埋めたもの）
af_map -TQd -v 1 -L map.log -c af_map.tcl        （= map/bin/yosys。af_map.tcl の 1 行目のコメントにある起動法）
    → OpenGDEMU.vqm（cycloneive_lcell_comb / dffeas / cycloneive_io_ibuf,obuf / alta_bram9k / alta_pllve のネットリスト）
（fixio.py：inout の ibuf + obuf を 1 個の alta_io にまとめる。§4-1）
af_run --mode NATIVE [--seed N]
    → 配置配線、タイミング報告（alta_db/*.rpt）、OpenGDEMU.bin / _slave.rbf / _master.bin / _master.prg / _master_as.prg / _sram.prg / _routed.v
```

- `af_map.tcl` の中身は `yosys -import` のあと、`memory_libmap -lib +/agm/common/brams_m9k.txt`、`techmap … brams_map_m9k.v`、`abc -lut 4`、
  `iopadmap`、`techmap … cells_map.v`、`write_verilog … .vqm` を順に呼ぶ Tcl。`synth_agm` という 1 個のコマンドは無い。
- 所要時間：`af_map` 約 7 秒、`af_run` 約 100 秒（配線表の読み込み 10 秒、配置 30 秒、配線 30 秒）、メモリ 1.4 GB。
- ピン割り当ては `OpenGDEMU.asf` に `set_location_assignment -to <ポート> <ピン名>`（Quartus と引数の順が逆。バスのビットは Tcl なので `{DC_DATA[3]}` と波括弧で囲む）。
  ピン名は `etc/AG16KDDF256.pinmap` の 1 列目（DDF256 のボール名に `PIN_` を付けたもの。2 列目は EP4CE15F23 側の対応ピン）。
  クロック入力の並び（J4 = CLK1、K1/L1 = CLK2/3、G16/G15 = CLK4/5、H16/H15 = CLK6/7、F9/E9 = CLK10/11、T14/R14 = CLK14/15）と
  DCLK = G1、DATA0 = G2、JTAG = H1/H2/J1/J2 は、`ag16k-config.md` §1-3 がマニュアルの画像から読んだ表と一致した（別の出どころでの裏づけ）。
- `af_run` は `.asf` を読む前に、全ポートについて「`Error: Pin …'s locatiion is not assigned.`」を出す（`.ve` ファイルを渡していないため。34〜66 件）。
  そのあと `.asf` が適用され、最後の集計は `0 fatals, 34 errors` でも終了コード 0 で生成物は出る。**この「Error」は無視してよい種類**だが、紛らわしい。
  `.ve` 形式（`<ポート> <ピン>` の 2 列）で渡せば消えると思われる（未確認）。
- 圧縮の切り替えは `.asf` に `set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION OFF`。**既定は圧縮あり（LZW）**。

## 3. 入力ファイル（リポジトリには入れていない。`~/supra-linux/work/in/`）

- `OpenGDEMU.v`：`fpga/src/OpenGDEMU.v` から `core_pll` のインスタンス（と `core_reset` / `core_clock`）を外しただけ（R32 の方針どおり。出力はどこにも使われていない）。ほかは同一。
- `Audio_PLL.v`：v5.5 と同じ名前・同じポート（`inclk0` → `c0`）で、中身は `alta_pllve`。パラメータは Supra 自身に作らせた：

  ```
  af_cmd -f pll.tcl      # 中で: load_architect -no_work -no_route -type AG16KDDF256 1 1 1000 1000
                         #        generate_pll Audio_PLL_PLLVE -pll_type PLLVE -input_freq 11.2896 -output_freqs {33.8688} -fb_mode 0
  → VCO 914.458 MHz、clkout0 33.869 MHz。CLKFB_HIGH/LOW = 0x27/0x28（÷81）、CLKOUT0_HIGH/LOW = 0x0C/0x0D + TRIM（÷27）、FEEDBACK_MODE = 4
  ```

  `clkfb` は `clkfbout` に直結、`pfden` と `resetn` は 1、位相調整・再構成の入力は 0 にした。
- `OpenGDEMU.sdc`：v5.5 の `.sdc` から、`create_clock` 2 本、`derive_pll_clocks`、`set_clock_groups -asynchronous`（48 MHz と 11.2896 MHz）、
  `set_max_delay` 5 本、`set_false_path -from DC_RSTn` を移したもの（`set_min_delay`・`derive_clock_uncertainty`・`set_time_format` は外した）。
- `OpenGDEMU.asf`：**仮のピン配置**（`mkasf.py` が生成）。クロック 2 本は専用クロック入力に、MCU 側 31 本と G1 側 33 本は、
  特別な機能の無いボールを pinmap の並び順に 2 つのかたまりで割り当てた。DDR_* と NC_* のピンは使っていない。

## 4. 見つかった問題と対処

### 4-1 inout が 2 つのパッドに分かれる（要対処。Supra 付属フローの不具合か、想定外の使い方）

`af_map.tcl` は inout の各ビットを `cycloneive_io_ibuf` と `cycloneive_io_obuf`（`oe` 付き）の 2 個にして `.vqm` に出す。
`af` はこの 2 個を**別々の I/O として別々のパッドに置く**（`Total Pins : 98`、`Warn: IO DC_DATA[0] is not assigned, placed at pin …` が同じ名前で 2 回、
配置後のネットリストでも座標が別）。`.asf` の位置指定も inout には効かない。入出力 2 ビットだけの小さな設計（`work/t1`）でも再現した。
このままだと `MCU_DATA` と `DC_DATA` の 32 本が壊れた（入力と出力が別ボールの）ビットストリームになる。

- 効かなかったこと：セル名を Quartus の `.vo` と同じ `\d[0]~input` / `\d[0]~output` にする（`work/t2` の最初の試行）。
- **効いたこと**：`.vqm` の中で、同じポートにつながる ibuf と obuf を 1 個の `alta_io`（`.padio` / `.datain` / `.oe` / `.combout`）に置き換える（`fixio.py`）。
  `af` はこれを 1 個の `alta_dio` に変換し、`oe`・`datain`・`combout` がつながった形で 1 パッドに置く。`Total Pins : 66`、位置指定も効く。
- 正攻法の候補（未検証）：`af_map.tcl` の `iopadmap … -tinoutpad` の写像先（`cells_map.v` の `$__tinoutpad`）を `alta_io` 1 個にする／
  上位ラッパで inout を `alta_io` として明示する／`OpenGDEMU.post_map.asf`（`af_map.tcl` が最後に source する）で Yosys のコマンドとして直す。
  Supra のツリーは書き換えたくないので、3 つ目か、今回の後処理スクリプトが現実的。

### 4-2 PLL

- **AG16KDDF256 の PLL は `alta_pllve` で配置配線が通った**（配置は座標 (0,1,0)、`Total PLLs : 1/4`、PLL の出力はグローバルクロックに載り、
  `Auto constraint PLL: create_generated_clock -name audio_pll_inst|pll|clkout0 -multiply_by 3 …` と自動で制約が付いた）。R32 の推測（`alta_pllv` か `alta_pllve`）のうち後者。
  `generate_pll` は `PLL` / `PLLX` / `PLLV` / `PLLVE` のどの型でもデバイスに関係なく Verilog を出すので、「生成できた」だけでは型の判定にならない。ほかの型が AG16K で拒否されるかは試していない。
- **PLL の入力ピンは選ぶ**：11.2896 MHz を J4（CLK1）や T14（CLK14）に置くと通り、G16（CLK4）に置くと
  `Error: Can not route to pin audio_pll_inst|pll|clkin.` で配線に失敗した（`work/b6`）。クロックピンを指定しない自動配置（`work/b1`）でも同じ失敗。
  PLL の位置は自動で、ピンと PLL の対応表は調べていない。**6.0 の基板で 11.2896 MHz がどのボールに来ているかで、使える PLL が決まる。**
- 未確認：`fb_mode 0`（`FEEDBACK_MODE = 4`）と `clkfb ← clkfbout` の結線が AG16K で正しい使い方か、ロックするか、VCO 914 MHz が範囲内か。
  Supra は警告を出さなかったが、実機でしか確かめられない。

### 4-3 FIFO（R39 の懸念）

`map.log` の `MEMORY_LIBMAP`：5 個とも `$__BRAM9K_TDP_` に写像された。

```
mapping memory OpenGDEMU.core|cdda|fifo|storage via $__BRAM9K_TDP_
mapping memory OpenGDEMU.core|dev0|read_fifo|storage via $__BRAM9K_TDP_      （8 個）
mapping memory OpenGDEMU.core|dev0|write_fifo|storage via $__BRAM9K_TDP_     （1 個）
mapping memory OpenGDEMU.core|dev1|read_fifo|storage via $__BRAM9K_TDP_      （2 個）
mapping memory OpenGDEMU.core|dev1|write_fifo|storage via $__BRAM9K_TDP_     （1 個）
```

合計 `alta_bram9k` 14 個（cdda が 2 個）。R39 の見積もり（14 M9K）と同じ。配置配線後のネットリストにも 14 個残っている。
`--target ag16k` の `SyncReadFIFO`（登録読み出し）のおかげで、非同期読み出しの問題は出ない。
`--target cyclone2` のコア（非同期読み出し）をこのフローに通すとどうなるかは試していない（試す理由が無い）。
注意：`af` が `Info: Adding byte enable net …PORT_A_WR_BE to write enable for ram …` を出している（バイトイネーブルを書き込みイネーブルに足した、の意）。動作が合っているかは実機かシミュレーションで見る。

### 4-4 そのほか

- `Warn: User constraints has 9.2% uncovered.`：制約の掛かっていない接続が約 9 %。`alta_db/coverage.rpt.gz` を見ると、同期リセットの配線、`MCU_RSTn`、
  `DC_ADDR` などの入力からの経路。v5.5 の `.sdc` で `get_ports → get_clocks` と書いた `set_max_delay` が、Supra では一部しか効いていない可能性がある。本番の制約を作るときに見直す。
- 同じ入力でも `.asf` の 1 行（圧縮の ON/OFF）を変えただけで配置結果とタイミングが変わった（b3 と b4）。再現性はシードと入力一式で決まると見て、生成物をコミットするときは R32 の方針どおり入力と出力のハッシュを残す。
- 未使用ピンの状態、DDR を休止させる信号（CKE・CS#）、I/O 規格・駆動能力は何も指定していない（既定のまま）。サイズにはほとんど効かないが、本番では要る（`g1-ag16k-port.md` §2-5）。

## 5. 資源とタイミング（`work/b4`、非圧縮、シード既定 666、クロック J4 / H16）

```
Packing Statistics
 Total      Logics : 2291/16624 ( 13%)
 Total        LUTs : 1980/16624 ( 11%)
 Total   Registers : 1089/16624 (  6%)
 Total  Block Rams :   14/   56 ( 25%)
 Total Multipliers :    0/   56 (  0%)
 Total        PLLs :    1/    4 ( 25%)
 Total        Pins :   66/  251 ( 26%)
 Global    Signals :    3/   20 ( 15%)   （CLK_11_2896_MHz、CLK_48_MHz、audio_pll_inst|pll_clkout0）
 Registers with synchronous reset : 1012
```

Yosys の `stat`（`map.log`）：`cycloneive_lcell_comb` 1,964、`dffeas` 1,089、`alta_bram9k` 14、`alta_pllve` 1、ibuf 57、obuf 41。
R39 の見積もり（YoWASP Yosys：LUT 2,821、FF 1,097、M9K 14）と比べて LUT は 3 割少なく、FF と M9K はほぼ同じ。

| ビルド | 条件 | 48 MHz 系 Fmax | 11.2896 MHz 系 Fmax | 最悪の経路（`→ DC_DATA`、制約 20 ns）の余裕 |
|--------|------|---------------:|--------------------:|-----:|
| b3 | 圧縮あり、シード 666、J4 / H16 | 97.9 MHz | 149.1 MHz | +2.95 ns |
| b4 | 非圧縮、シード 666、J4 / H16 | 90.5 MHz | 158.6 MHz | +2.68 ns |
| b5 | 非圧縮、シード 1、J4 / H16 | 93.4 MHz | 154.4 MHz | +2.11 ns |
| b7 | 非圧縮、シード 7、T14 / F9 | 92.0 MHz | 174.8 MHz | +2.59 ns |

- 最悪の経路は M9K の読み出し → バイパスの選択 → 出力の選択 → `DC_DATA` のパッド（クロックからパッドまで約 17.3 ns）。v5.5 の「16 ns が限界」と同じ形の経路。
- ホールドの最悪は +0.29〜+0.33 ns（48 MHz 系）。
- **仮ピンでの値**なので、実ピンでは変わる。傾向として「48 MHz に対して 2 倍近い余裕、G1 の出力は 20 ns に収まる」。

## 6. ビットストリームの大きさ

測り方：リポジトリと同じ設定（`mcu/netgd/build.rs` の `XZ_FILTER`、`common/lzma` のデコーダが前提にする値）で

```
xz --format=raw --lzma1=preset=9e,dict=4KiB,lc=0,lp=0,pb=0 -c <file> | wc -c
```

| ビルド | ファイル | 元の大きさ | LZMA 後 |
|--------|----------|-----------:|--------:|
| b4（非圧縮） | `OpenGDEMU.bin` | 552,352 | 37,368 |
| b4（非圧縮） | **`OpenGDEMU_slave.rbf`**（`build.rs` が読む形） | **552,352** | **37,727** |
| b4（非圧縮） | `OpenGDEMU_master.bin` | 552,366 | — |
| b5（非圧縮、シード 1） | `.bin` / `_slave.rbf` | 552,352 | 38,277 / 38,609 |
| b7（非圧縮、シード 7、別のクロックピン） | `.bin` / `_slave.rbf` | 552,352 | 38,199 / 38,534 |
| b3（AGM 圧縮 = 既定） | `OpenGDEMU.bin` / `_slave.rbf` | 255,887 | 71,381 / 71,959 |
| b3（AGM 圧縮） | `OpenGDEMU_master.bin` | 255,901 | 71,391 |
| （参考）b4 の `.rbf` を辞書 8 KiB / 64 KiB で | | 552,352 | 37,582 / 37,542 |
| （参考）入出力 2 ビットだけの設計、AGM 圧縮 | `work/t2/tio.bin` | 272,300 | — |

ファイルの関係（b4 で確認）：

- `_slave.rbf` は `.bin` の各バイトのビット順を逆にしたもの（全バイト一致）。`ag16k-config.md` §2-1 の記述どおり。
- `_master.bin` は `.bin` の前に 14 バイト `55 55 00 00 0e 00 00 … 00` を付けたもの。552,366 B は rodinia の `blinky16k.bin` と同じ大きさ（R32）。
- `.bin` の先頭は `96 7e 3c 5a | 01 50 00 10 | 00 00 ff ff | 00 03 …`（非圧縮）／`… | 55 03 …`（圧縮）。デバイス ID `0x01500010` と圧縮の印 `0x55` は `ag16k-config.md` §2-2 の記述と合う。
- 非圧縮の `.bin` は 552,352 B のうち 458,998 B が 0x00。LZMA がよく効く理由。
- **往復の確認**：b4 の `.rbf` と `.bin` を上の `xz` で圧縮し、`common/lzma` の `Decoder`（スクラッチパッドに作った小さな Cargo プロジェクトから path 依存で使用）で展開して、
  元と全バイト一致・エラー無しを確認した（37,727 → 552,352、37,368 → 552,352）。
- SHA-256（b4）：`.bin` `0d2c9647…121a43`、`_slave.rbf` `66ae7eed…11cfd0`、`_master.bin` `d642ba28…f1646c`。

読み方：

- **AGM の圧縮（LZW）は埋め込みには不利**。生で 250 KB、LZMA をかけても 71 KB（`ag16k-config.md` §2-4 の「LZW の出力は LZMA でほとんど縮まない」という推測より良いが、非圧縮 → LZMA の約 2 倍）。
  しかも**ほぼ空の設計でも 272 KB** で、設計の規模に比例しない。SD に置く場合でも、非圧縮 → LZMA のほうが小さい。
- 非圧縮 → LZMA は配置によって 37.4〜38.6 KB（今回の 3 回の幅 約 1.2 KB）。**実ピン・コアの変更でも 40 KB 前後に収まると見てよい**が、±数 KB は動きうる（推測）。
- 辞書を 8 KiB にしても −145 B（0.4 %）。R81 の案 d（約 −2 % の見込み）は AG16K では効かない。

### R81 の上限との比較（`flash-budget-2e.md` §3 の表。フラッシュ側は R81 時点の数字）

| 構成 | 埋め込みの上限 X（`sd-fast` あり） | 同（伸びしろ 8 KB） | 37,727 B は |
|------|----------------:|-----------:|------|
| 現状（`opt-level="s"`、削減なし） | 51,364 | 43,172 | **入る（伸びしろ込みでも +5.4 KB）** |
| 案 1（`list_root_dir` 削除）のみ | 53,412 | 45,220 | 入る |
| 案 1 + 2（`z`） | 61,092 | 52,900 | 入る（+15 KB） |

判断表の条件（「実サイズが約 53 KB 以下、かつ `list_root_dir` の削除と `opt-level="z"` を入れた場合に限って検討」）は、
**サイズについては大きく満たす。しかも `z` は必須ではなくなった**（入れれば、その分がほかの機能の伸びしろになる）。

## 7. 埋め込みか SD か（R66）についての見解

- **サイズの面では、埋め込み（案 a）と「SD 優先・埋め込みは予備」（案 c1）がどちらも成り立つ。** SD カード無しで FPGA が起きる構成（本家と同じ挙動）を選べる。
- 残る論点：
  1. **流す時間**：非圧縮は 4,418,816 bit（552,352 × 8）で固定。v5.5（約 1.27 Mbit）の 3.5 倍。`ag16k-config.md` §4-3 の手当て（1 秒前後）が前提になる。
     AGM 圧縮なら 2.05 Mbit だが、フラッシュには載らない（71 KB）。**「埋め込み = 非圧縮を LZMA」で決まり**、時間は PS の送り方で稼ぐしかない。
  2. **実ピンでの再測定**：仮ピンなので、6.0 のピン配置が分かったら同じ手順で測り直す（数 KB の変動を見込む）。
  3. フラッシュ側の数字は R81 以降のコード追加（LFN など）で動いている。埋め込むときに `board-v60,resident,sd-fast` のリンクで確かめる。
- 決定はユーザー（判断表 2026-11-20 の「Supra で実際のサイズを測ってから決める」の材料がそろった、という位置づけ）。

## 8. 未確認・限界

- 実機での動作は一切未確認（コンフィグが通るか、PLL がロックするか、I/O が正しいか、タイミングが足りるか）。
- ピン配置は仮。DDR の休止、未使用ピンの扱い、I/O 属性は未設定。
- 生成した `.bin` を Supra 以外のツール（rodinia の `agm-unpack.py` など）で検査していない。
- `fixio.py` 後のネットリストが論理的に正しいことは、Supra の配置後ネットリストで `alta_dio` の結線（`datain`・`oe`・`combout`）を目で見ただけ。等価性検証やゲートレベルのシミュレーションはしていない。
- ライセンスが「なぜ」通ったか（別マシン向けのノードロックではなかったのか、検査が緩いのか）は調べていない。調べる必要も無いと判断した。
- 通信の有無は `strace` で見ていない（§1）。
- Supra 2026.03 の 1 版だけの結果。

## 9. 再現手順（概要）

前提：WSL に Supra を展開済み（`~/supra-linux/x/Supra-2026.03.b0/`）。スクリプト一式は `~/supra-linux/work/in/` に残してある
（`ns.sh`・`build.sh`・`mkasf.py`・`fixio.py`・`pll.tcl`・`OpenGDEMU.v`・`Audio_PLL.v`・`OpenGDEMU.sdc`・`OpenGDEMUCore_ag16k.v`）。

```sh
# 0) コアの Verilog（Windows 側）
cd fpga/amaranth && amaranth_env/Scripts/python.exe main.py --target ag16k     # out/OpenGDEMUCore_ag16k.v。CRLF は LF に直して WSL へ

# 1) ネットワーク無しのラッパ（in/ns.sh）
S=$HOME/supra-linux/x/Supra-2026.03.b0
exec unshare -rn env SUPRA=$S PATH=$S/bin:/usr/bin:/bin "$@"

# 2) ビルド（in/build.sh <dir> <11.2896MHz のピン> <48MHz のピン> [.asf に足す 1 行]）
cd ~/supra-linux/work/<dir>          # 入力 4 ファイルをコピー済み
python3 -I ../in/mkasf.py $S/etc/AG16KDDF256.pinmap PIN_J4 PIN_H16 > OpenGDEMU.asf      # 仮ピン
echo 'set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION OFF' >> OpenGDEMU.asf  # 非圧縮にする
../in/ns.sh af_setup --design OpenGDEMU --device AG16KDDF256 --verilog OpenGDEMU.v Audio_PLL.v OpenGDEMUCore_ag16k.v
../in/ns.sh af_map -TQd -v 1 -L map.log -c af_map.tcl
python3 -I ../in/fixio.py OpenGDEMU.vqm OpenGDEMU.fixed.vqm && cp OpenGDEMU.fixed.vqm OpenGDEMU.vqm   # inout を alta_io 1 個に
../in/ns.sh af_run --mode NATIVE [--seed N]

# 3) 測定
grep -A12 'Packing Statistics' run.out; cat alta_db/fmax.rpt
xz --format=raw --lzma1=preset=9e,dict=4KiB,lc=0,lp=0,pb=0 -c OpenGDEMU_slave.rbf | wc -c
```

`fixio.py` の要点（`.vqm` のテキスト処理）：`cycloneive_io_ibuf X (.i(PAD), .ibar(1'h0), .o(N));` と `cycloneive_io_obuf Y (.i(D), .o(PAD), .oe(E));` が
同じ `PAD` を持つとき、2 つを `alta_io \PAD~io (.padio(PAD), .datain(D), .oe(E), .combout(N));` 1 個に置き換える（今回は 32 組）。

作業フォルダの中身：

| フォルダ | 内容 |
|----------|------|
| `work/b1` | 最初の試行（ピン指定なし）。PLL の入力が配線できず失敗。資源の報告は出る |
| `work/b2` | ピン指定あり、inout の修正なし（98 ピン。**壊れたビットストリーム、使わない**） |
| `work/b3` | inout 修正あり、AGM 圧縮（既定）。255,887 B |
| **`work/b4`** | **inout 修正あり、非圧縮。552,352 B。この文書の基準** |
| `work/b5`・`b7` | b4 のシード違い・クロックピン違い（ばらつきの確認） |
| `work/b6` | 11.2896 MHz を G16 に置いた失敗例（PLL に届かない） |
| `work/t1`・`t2` | inout の問題を切り分けた 2 ビットの試験設計 |
| `work/probe` | `generate_pll` の出力（4 種の PLL） |

## 10. 関連

- `docs/design/g1-ag16k-port.md` §2（フロー・PLL・メモリ）、§3-2（サイズの見積もり）― 本書で実測に置き換わる部分がある
- `docs/research/ag16k-config.md` §2（ファイルの種類と圧縮）、§4-3（流す時間）
- `docs/research/flash-budget-2e.md` §3（埋め込みの上限）、`docs/design/g1-bitstream-storage.md`（R66）
- `docs/research/supra-static-analysis.md`（R92 / R95）
- LOG：R32、R39、R66、R69、R81、R92
