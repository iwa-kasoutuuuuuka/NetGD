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
- `fixio.py` の後のネットリストの等価性（Supra の配置後ネットリストを目で見ただけ）。
- DDR の休止、未使用ピンの扱い、I/O 規格・駆動能力（何も指定していない）。
- Supra 2026.03.b0 以外の版。
