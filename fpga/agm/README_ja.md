# fpga/agm — AG16K（GDEMU 6.0）向けビットストリームのビルド

AGM Supra 2026.03（Linux 版）で、NetGD のコアを AG16KDDF256 向けにビルドする手順とスクリプト。
元になった調査は `docs/research/ag16k-supra-build.md`（LOG R99）。

**実機では何も確かめていない。** ここで確かめたのは「Supra がエラーなくビットストリームを出す」ことと、その大きさ・資源・タイミングの報告だけ。

## いちばん大事なこと：ピン配置は仮

GDEMU 6.0 の FPGA のピン配置は分かっていない。`pins/PROVISIONAL.asf` は、フローを動かして大きさを測るためだけの**仮の割り当て**で、
6.0 の配線とは関係がない。これで作ったビットストリームを基板に流すと、すべての I/O が間違ったボールに出る。

そのため `build.sh` は次のように動く。

| ピンファイル | 出力 | ファームが拾うか |
|--------------|------|------------------|
| 指定なし（= `pins/PROVISIONAL.asf`）、または印 `NETGD-PROVISIONAL-PINS` を含むファイル | `output/OpenGDEMU_ag16k-provisional.rbf` | **拾わない** |
| `--pins <実際のピンファイル>`（印を含まず、66 ポートすべてを 1 回ずつ割り当てたもの） | `output/OpenGDEMU_ag16k.rbf` | 拾う（`mcu/netgd/build.rs` の `RBF_V60`） |

- `-provisional` の付いたファイルを `OpenGDEMU_ag16k.rbf` に名前を変えて置かないこと（ファームの大きさを測る実験を除く。測ったら消す）。
- `output/` は `.gitignore` に入れてある。ビットストリームはコミットしない。
- 実際のピンファイルを作るには、6.0 の基板の導通確認（`docs/research/user-tasks.html`）が要る。作ったら `pins/` に置く（例：`pins/gdemu60.asf`）。
  書式は `set_location_assignment -to {DC_DATA[3]} PIN_xx`（Quartus と引数の順が逆。バスのビットは波括弧で囲む）。
  ポート名の一覧は `pins/PROVISIONAL.asf` にある。11.2896 MHz は PLL に届くクロック入力に置く必要がある（J4・T14 は通り、G16 は通らなかった）。

## 使い方（WSL / Linux）

```sh
# 0) コアの Verilog（Windows 側。コミット済みのものをそのまま使うなら不要）
cd fpga/amaranth && amaranth_env/Scripts/python.exe main.py --target ag16k

# 1) ビルド（WSL の中で）
SUPRA_HOME=$HOME/supra-linux/x/Supra-2026.03.b0 fpga/agm/build.sh              # 仮ピン
SUPRA_HOME=... fpga/agm/build.sh --pins fpga/agm/pins/gdemu60.asf              # 実ピン（まだ無い）
```

| オプション | 意味 |
|------------|------|
| `--pins FILE` | ピン割り当て（`.asf`）。省くと仮ピン |
| `--seed N` | 配置配線のシード（省くと Supra の既定） |
| `--core FILE` | コアの Verilog（既定 `../amaranth/out/OpenGDEMUCore_ag16k.v`） |
| `--work DIR` | 作業フォルダ（既定 `fpga/agm/build/`。毎回消して作り直す） |
| `--timeout S` | 配置配線を S 秒で打ち切る（既定 1,200）。配線が最後の衝突を解けずに回り続けることがある。そのときはもう一度実行するか、シードを変える |
| `--supra-retiming` | af_map の RETIMING 段を切らずに残す。**試験専用**（下の「af_map の RETIMING 段」）。報告の `retiming` 行が `ON` になる |

- 入力：`fpga/amaranth/out/OpenGDEMUCore_ag16k.v`、`src/OpenGDEMU.v`、`src/OpenGDEMU.sdc`、ピンファイル。
- 出力：`output/<名前>.rbf`（非圧縮、552,352 B、PS 用にビット順を逆にした `_slave.rbf`）と `output/<名前>.txt`
  （SHA-256、LZMA 後の大きさ、入力のハッシュ、資源、Fmax）。
- 所要時間は 2〜3 分、メモリ 1.4 GB ほど。作業フォルダを WSL のファイルシステム（`--work ~/…`）に置くと少し速い。
- 必要なもの：`unshare`（util-linux）、`timeout`・`sha256sum`（coreutils）、`python3`、`xz`。

## 安全上の決まり（スクリプトが守っていること）

- Supra の実行ファイルは**必ず `unshare -rn`（ネットワークの無いユーザー名前空間）の中**で動かす。外す手段は用意していない。
- 実行するのは `af_cmd`・`af_setup`・`af_map`・`af_run` の 4 つだけ。GUI、ダウンローダ、JTAG・USB の道具、ライセンス関係のコマンドは呼ばない。`sudo` も使わない。
- Supra のツリーには書き込まない。Supra のファイルをリポジトリに写さない。
  Supra が作るファイル（`af_*.tcl`、`.vqm`、PLL の Verilog など）は作業フォルダ（`build/`、`.gitignore` 済み）にだけ置く。

## ファイル

| ファイル | 中身 | 出どころ |
|----------|------|----------|
| `build.sh` | 全体の流れ（下の図） | NetGD |
| `src/OpenGDEMU.v` | 最上位。`fpga/src/OpenGDEMU.v` から、どこにも使われていない `core_pll` を外しただけ | NetGD（本家の最上位が元） |
| `src/OpenGDEMU.sdc` | v5.5 の `.sdc` から必要な制約を移したもの | NetGD（本家の `.sdc` が元） |
| `pll.tcl` | Supra の `generate_pll` を呼ぶ 3 行（11.2896 MHz → 33.8688 MHz、`alta_pllve`） | NetGD |
| `mkpll.py` | `generate_pll` の出力からパラメータの値だけを読み、v5.5 と同じ名前・ポートの `Audio_PLL` を作業フォルダに書く | NetGD |
| `fixio.py` | `.vqm` の中で、inout の `ibuf` + `obuf` を 1 個の `alta_io` にまとめる（これをしないと入力と出力が別のボールになる） | NetGD |
| `mkasf.py` | 仮のピン配置を作る（`pins/PROVISIONAL.asf` の本体を作ったスクリプト。Supra の pinmap を読む） | NetGD |
| `pins/PROVISIONAL.asf` | **仮の**ピン配置 | `mkasf.py` の出力 |

`Audio_PLL.v` をリポジトリに置いていない理由：中身のほとんどが Supra の `generate_pll` が出した値の並びなので、
Supra の生成物として扱い、ビルドのたびに作らせることにした。置いてあるのは「どう呼ぶか」（`pll.tcl`）と「どう包むか」（`mkpll.py`）だけ。

```
pll.tcl ──(af_cmd)──▶ build/pll/Audio_PLL_PLLVE.v ──mkpll.py──▶ build/Audio_PLL.v
src/OpenGDEMU.v, Audio_PLL.v, OpenGDEMUCore_ag16k.v
   ──(af_setup)──▶ af_map.tcl ほか（Supra のひな形から）
   ──(af_map = Yosys)──▶ OpenGDEMU.vqm ──fixio.py──▶ OpenGDEMU.vqm（inout 32 本を 1 パッドに）
   ──(af_run --mode NATIVE)──▶ OpenGDEMU_slave.rbf ──検査──▶ output/OpenGDEMU_ag16k[-provisional].rbf
```

`build.sh` が最後に確かめること：`.rbf` が 552,352 B で先頭が `69 7e 3c 5a`、ピン数が 66、Supra が PLL を 3 倍と報告していること、
「ピンの位置が未割り当て」以外の Error が無いこと。どれかが外れたら出力を置かずに止まる。

## af_map の RETIMING 段（切ってある）

Supra 2026.03.b0 の `af_map.tcl` は既定で dfflegalize の前に `abc -markgroups -dff -D 100`（RETIMING）を走らせる。
この Yosys 0.61 の `abc -dff` は、フリップフロップを「毎クロック Q ← D」の素のラッチとして ABC に渡し、イネーブルと同期リセットは後で付け直す。
ABC の `scorr`・`retime` はその誤ったモデルの上で回路を変えるので、イネーブルや同期リセットを持つ FF が壊れうる
（最初はピン探しの設計で見つかった。`fpga/pinfind/README_ja.md`）。このコアで確かめたこと（2026-11-22、マップ前後のネットリストを書き出して比較・シミュレーション）：

- `cdda.pair_valid`（イネーブルの下で定数 1 を入れ、同期リセットで 0 に戻す）が消される。音声 FIFO が一度も読まれず CDDA は無音、
  アンダーフローも数えない。マップ後の `.vqm` を上位から動かすと RETIMING ありでこうなり、なしでは RTL とトレースが一致する。
- 「イネーブル中だけ効く同期リセット」（`$sdffce`）が「いつも効く同期リセット」（`$sdffe`）になる。該当は `cdda.shift[0]` と `cdda.sdat` の 2 個だけ。
  `shift[0]` は右チャンネルの最下位ビットが常に 0 になる（この 2 個だけを書き換えたシミュレーションで確認）。`sdat` は出力に差が出ない。
- 電源投入時の値が動く（`cdda.phase` の初期値。自走カウンタなので害は無い）。

`build.sh` は af_setup が作る空の `OpenGDEMU.pre_map.asf` に `set RETIMING "None"` を書き（`af_map.tcl` は既定値を決めた後でこれを読む）、
`map.log` で DFFLEGALIZE より前に ABC が走っていたら止まる。報告（`output/<名前>.txt`）の `retiming` 行に結果を書く。

| 2026-11-22、仮ピン | LUT | FF | 48 MHz 系 Fmax | 11.29 MHz 系 Fmax | 余裕 | LZMA 後 |
|---|---:|---:|---:|---:|---:|---:|
| RETIMING なし（既定）、シード既定 / 1 | 2,241 | 1,115 | 97.9 / 92.7 MHz | 125.4 / 123.9 MHz | +3.34 / +2.96 ns | 42,094 / 42,100 B |
| RETIMING あり（`--supra-retiming`）、シード既定 / 1 | 2,060 | 1,107 | 90.6 / 91.3 MHz | 149.5 / 176.1 MHz | +2.03 / +3.08 ns | 40,502 / 40,509 B |

切ると LUT が 9 % 増え、LZMA 後は 1.6 KB 大きくなる（これまでの 36.7〜40.0 KB は RETIMING ありの値）。タイミングは悪くならない。

## 再現性について

**同じ入力でも、Supra の出力はビット単位では同じにならない**（`docs/research/ag16k-supra-build.md` §11-3。シードを固定すれば同じになるかは未確認）。
再現できるのは「手順と検査」までで、`.rbf` のハッシュではない。配置が変わると LZMA 後の大きさも動く（これまでの幅は 36.7〜40.0 KB）。ファームに入れるビットストリームを決めたら、`output/<名前>.txt` のハッシュを記録に残すこと。

## ファームへの埋め込み

`mcu/netgd/build.rs` は、`board-v60` のビルドで `fpga/agm/output/OpenGDEMU_ag16k.rbf` があれば LZMA で固めて埋め込み、無ければ
「ビットストリーム無し」のファーム（起動時に LED 5 回点滅）を作る。仮ピンのビルドは別の名前で出るので、既定のビルドには影響しない。

```sh
cd mcu/netgd && DEFMT_LOG=off cargo build --release --no-default-features --features board-v60
```

大きさと PS で流す時間の見積もりは `docs/research/ag16k-supra-build.md` §11〜§13。

## 未確認

- 実機での動作すべて（コンフィグが通るか、PLL がロックするか、I/O、タイミング）。
- `fixio.py` の後のネットリストの等価性（Supra の配置後ネットリストを目で見ただけ）。マップ直後の `.vqm`（fixio 前）は、
  修正したセルモデルで上位から短い刺激（MCU レジスタ読み、CDDA 有効化と 16 語、ATA レジスタ読み）を与え、RTL とトレースが一致した（2026-11-22）。配置配線後は未確認。
- DDR の休止、未使用ピンの扱い、I/O 規格・駆動能力（何も指定していない）。
- Supra 2026.03.b0 以外の版。
