# fpga/pinfind — GDEMU 6.0 のピン探索（方法 2）

`docs/design/g1-ag16k-port.md` §2-4 の「方法 2：ピン探索用ビットストリーム」の実装。
6.0 基板の FPGA（AGM AG16KDDF256）のどのボールが、MCU（ATSAM3U2E）のどのピン・GD-ROM コネクタ（CN1）のどのピンにつながっているかを、
FPGA 自身にボールの番号を名乗らせて調べる。結果は `fpga/agm` の本番ビルド用のピンファイル（`.asf`）に機械的に変換できる。

**実機では何も確かめていない。** この PC には Supra も WSL も無いので、ビットストリームも一度も作っていない。
`build.sh` の検査（§3-3）も、Supra の出力の形式を知らないまま書いたもので、一度も Supra の出力に当てていない。
確かめたのは、Amaranth のシミュレーション・ホストのテスト・ファームのビルドまで（§8）。

## 0. 実行の前に（必ず読む。LOG R143）

> **基板は替えがきかない。** 純正ファームはバックアップが無ければ戻せない。迷ったら進めない。

> **最初の実機実行（段階 0）の手順は [`FIRST_RUN_ja.md`](FIRST_RUN_ja.md) のチェックリストに順番に並べてある。段階 0 / 1 のファームは `tools/pinfind/mkstage.ps1 -Stage 0|1` で作る（ファイル名に段階とビットストリームの SHA-256 の先頭 8 桁が入る。§3-4）。**

### 0-1 最初の実機実行の前に必須なこと（どれか 1 つでも欠けたら走らせない）

1. **純正ファームのバックアップ**：`tools/fwbackup/README_ja.md` のとおり `check` → `UNLOCKED_2E` なら `dump` → 照合。
   **`LOCKED` なら実行しない**（純正は戻せない。続けるかはユーザーの判断で、2 枚目の基板を勧める）。
   ロックされた基板（R177、6.0 はロック前提）で、ユーザーが ERASE を決めた場合だけは、バックアップ無しで進む：
   `FIRST_RUN_ja.md` の手順 B'、`docs/research/gdemu60-bringup.md`（ERASE の後は SWD の確認 → 何も駆動しない `census` → 段階 0）。
2. **導通確認**（無通電、テスター）：PS の 5 本（PA23 = DATA0 = G2、PA24 = DCLK = G1、PA25 = nCONFIG = H3、PB1 = CONF_DONE = F16、PB2 = nSTATUS = F3）、
   MSEL1（F15）が PS の側（Low）に固定されていること、LED のピン（PB18）。今のファームは v5.5 の割り当てを仮定している（`cyclone::pinmap`）。
   **nCONFIG が PA25 でなければ、ファームは FPGA を止められない**ので、ここは必ず確かめる。
3. **基板の外での給電の予行**（R143）：純正ファームのまま、無通電で CN1 の電源ピン・U4・Q1・MCU の VDDIO・SWD の 1 番の導通と、各レールと GND の間の抵抗を記録 →
   Dreamcast に付けた状態でレールの電圧を基板上で測る → CN1 の電源ピンに同じ電圧を**電流制限つきの電源**（100〜150 mA から）で入れ、
   **純正ファームのまま基板の外で起動する**ことを確かめ、電流と電圧の基準値を取る。12 V に触れない、制限の無い電源を使わない。
4. **実行中の条件**：電流制限つきの電源で給電し、電流を記録する。**Dreamcast には付けない**。**SD カードは抜く**。
   デバッガで CPU を止めない（止めている間は nCONFIG を下げる処理も動かない。§6）。
5. ファームとビットストリームは段階ごとに作り直す（下の 0-2）。前の段階で何か 1 つでも想定と違えば、次に進まない。

### 0-2 段階を踏む（段階 0 → 1 → 2 → 3）

| 段階 | 作り方 | 確かめること | 次に進む条件 |
|------|--------|--------------|--------------|
| **0：出力ゼロ** | `mklist.py`（全ボール）→ `gen.py --stage0` → `build.sh` → ファーム | `PF FPGA configured`（CONF_DONE が上がる）、電流が予行の基準から大きく増えない、`PF across configuration: 0 watched line(s) changed`、20 秒の保持中に `PF diff` が出ない | 3 つとも想定どおり |
| **1：数本だけ** | `mklist.py --max-drive 4`（残りは hold）→ `gen.py`（既定）→ `build.sh` | `PF end` まで普通に終わる（4 本が MCU につながっていなければ `PF new` は出ず、120 秒の期限で終わる）、電流の変化、`PF stuck-low` が出ない | 想定どおり |
| **2：全ボール** | `mklist.py`（全ボール）→ `gen.py`（既定）→ `build.sh` | `PF new` / `PF hit` で MCU 側の対応が出る | — |
| **3：クロック** | 段階 2 と同じ一覧で `gen.py --probes` | `PF clk` で 48 MHz・11.2896 MHz のボールが分かる | — |

- 段階 0 は FPGA が何も駆動しない設計で、ファームの安全装置（期限・GPBR・nCONFIG）と PS のピン、未使用ピンの扱いを確かめる回。
  ここで `PF diff` が出たら、Supra が未使用ピンや hold のピンを出力にしている疑いがある。先に進まない。
- 段階 1 で走査するボールは一覧の先頭の数本（pinmap の並び順）。どのボールかは一覧（`balls.txt`）で分かる。
- 各段階の RTT のログ（`rtt.log`）と電流の記録、`output/pinfind_ag16k.txt` を残す。
- CN1 側（ロジアナ）は §5。MCU 側の段階 2 が済んでから、別の回で行う。

## 1. しくみ

```
Supra の pinmap ──mklist.py──▶ build/balls.txt（ボール名の一覧。コミットしない）
                                   │
          hw/pinfind.py ──gen.py──▶ build/PinFind.v・PinFindCore.v・PinFind.asf・PinFind.sdc・params.txt・balls.txt
                                   │
                         build.sh（WSL + Supra）──▶ output/pinfind_ag16k.rbf・.params・.txt（コミットしない）
                                   │   （配置配線の後に checkpnr.py：実際のボール配置を一覧と照合）
   mcu/netgd --features pinfind ◀──┘ （ビットストリームを埋め込み、.params から動作を決める）
            │ SWD で書き込み、RTT でログを取る
            ▼
        rtt.log ──┐
   ロジアナ CSV ─la_csv─▶ la.txt ──┼──pins2asf.py──▶ fpga/agm/pins/gdemu60.asf
                                   ┘
```

FPGA 側（`hw/pinfind.py`）がボール `k` ごとにすること：

1. **LISTEN**：全出力を切ったまま `listen` ティック、ボール `k` を見る。**一度でも Low を読んだら、そのボールは飛ばす**
   （プルアップ以外の何かが Low に引いているか、動かしている。クロックバッファの出力、駆動中のバス、GND に直結したボールなど）。
2. **FRAME**：ボール `k` だけ出力を有効にして、番号 `k` のフレームを `frames` 回送る（既定 2）。
3. **NEXT**：出力を切って次のボールへ。最後のボールの次で 1 周。`max_loops` 周（1〜8、既定 1）で止まり、以後は全出力オフのまま。

同時に出力になるボールは常に 1 本以下（シミュレーションで毎クロック確かめている）。
加えて**駆動の見張り**：1 本のボールが FRAME にいる時間をクロック数で数え、フレームの最長（から計算した上限）に達したら、その場で出力を切って止まる（以後は出力しない）。
順序回路が FRAME から抜けない場合（不具合や誤動作）への備えで、絶対時間の上限ではない（FPGA のクロックの周波数そのものが分からないため。§6）。

### フレームの形（形式 1。`common/pinfind/src/lib.rs` が正）

1 ティック T = FPGA のクロック 2^prescale 個。「アクティブ」= Low を出す、「アイドル」= High（オープンドレインでは手を離してプルアップに任せる）。

```
アイドル | SYNC: アクティブ 8T | アイドル 1T | ビット: アクティブ 1T（0）か 3T（1）+ アイドル 1T | … | アイドル 4T 以上
```

- 中身は MSB から：`idx`、`~idx`（番号とその反転）。クロック計測つき（`--probes`）なら続けて `hdr`（`div_bits << 4 | win_bits`）、
  クロックボールごとに 16 bit のカウント、最後に全バイトの和（下位 8 bit）。
- 受け側は SYNC とその直後のアイドル（計 9T）から T を測り、以後もビットごとに追従する。比べるのはパルス幅どうしだけなので、
  **FPGA のクロックが数百 kHz でも数百 MHz でも、ゆっくり揺れても同じに読める**（`common/pinfind/tests/decode.rs`、1 ティック 3〜1,000,000 単位）。

### クロックボール（入力専用）の見つけ方

専用クロック入力（J4・K1・L1・G16・G15・H16・H15・F9・E9・T14・R14）は出力にできないので、走査では見つからない。
段階 3（`gen.py --probes`）では各クロックボールで小さなカウンタを回し、その最上位ビットの変化回数を一定のティック数で数えてフレームに載せる（クロックプローブ）。
受け側は自分で測った T から周波数に直す（`pinfind::clock_hz`）。`--probes` を付けないとき、クロックボールは hold（入力）として設計に残る。

注意：pinfind のファームは MCU の PCK0（PA21、48 MHz の出どころの候補）を出さない（`init` が何も設定しない）。
48 MHz が MCU から来ている場合、そのボールは段階 3 でも止まって見える（R143。PCK0 だけを出す版は未実装）。

### FPGA のクロック源（`--clock`）

| `--clock` | 中身 | 未確認の点 |
|-----------|------|-----------|
| `osc`（既定） | AGM の内蔵オシレータ `alta_osc`（`i_osc_enb`、`o_osc`）。Supra のプリミティブ集に名前がある（Supra のファイルはリポジトリに入れていない）。シミュレーションモデルは `i_osc_enb` が 0 のときだけ動くので、Low で有効として 0 を入れた | AG16KDDF256 で配置できるか、極性、周波数（数 MHz らしいが公表値は無い） |
| `ring` | LUT（`cycloneive_lcell_comb`）31 段のリングオシレータ ＋ 2 分周 | `af` が組み合わせループを配置配線するか。周波数（5〜200 MHz と見ている） |
| `ball:PIN_G1` | ボールをクロック入力に使う。G1 = DCLK。**このときだけ** `pinfind` ファームは構成後も PA24 を出力のまま振る（`params.txt` の `toggle_dclk 1` を `build.rs` が読む） | DCLK を構成後にユーザー入力として使えるか。Supra が G1 へのユーザー信号の割り当てを許すか |

- 既定は `osc`、だめなら `ring`、それもだめなら `ball:PIN_G1`。
- **`ball:` のクロックには残るリスクがある**：MCU が止まる（デバッガで止める、CPU が固まる）と FPGA のクロックも止まり、フレームの途中の 1 本が Low のまま固定される。
  ファームは 500 ms 以上 Low のままの線を見つけると止める（§6）が、MCU 自体が止まった場合は何もできない。`osc`・`ring` が使えない場合の最後の手段にする。
- `ball:PIN_xx`（G1 以外。例：方法 1 で分かった 11.2896 MHz のボール）では DCLK は振らない。そのボールは一覧の hold か clock にある必要がある。
- `--clkctrl`（既定はオフ）はクロックを明示的な `alta_gclksw`（`ena` = 1）経由でグローバル網に載せる。**Supra 2026.03.b0 では使えない**：
  `cycloneive_clkctrl` は af_map が知らない（`NO_ALTA_GCLKSW` のときだけの blackbox）、`alta_gclksw` は af_run が「no corresponding resource」と言って落ちる。
  付けなくても af_run が自分で GCLK を作る（`Created GCLK cell for net clk`、Global Signals 1/20）。
- 最初の Supra のビルド（2026-11-22）で `osc`（`alta_osc` がマップ後・配置配線後のネットリストに残る、Supra の推定 Fmax 70〜76 MHz。Supra は `alta_osc` を 1 MHz と仮定して解析する）と `ring`（31 段が配置配線後も残る）はどちらも最後まで通った。周波数・極性は実機でしか分からない。

ティック（`--prescale`）の既定：`osc` 10、`ring` 12、`ball` 6（R143：オープンドレインの立ち上がりのため **T ≥ 20 µs**。
`osc` が 51 MHz 以下、`ring` が 204 MHz 以下、`ball:PIN_G1` ではファームのループが毎秒 320 万回以下なら満たす）。
ログの `PF unit=<サイクル数>` から FPGA のクロックが分かる：f = 2^prescale × 96,000,000 / unit。
`PF act`（変化はあるがフレームにならない）ばかりなら、prescale を上げて作り直す。

## 2. ファイル

| ファイル | 中身 |
|----------|------|
| `hw/pinfind.py` | 本体（Amaranth）：LISTEN / FRAME / NEXT の順序回路、フレームの符号器、クロックプローブ、駆動の見張り |
| `src/pf_clock.v` | クロック源（`alta_osc` / リング / ボール）と、任意の `alta_gclksw`（Supra 2026.03.b0 では不可） |
| `portmap.py` | 「どのボールをどのポートに置くか」の規則。`gen.py` と `checkpnr.py` が共有する |
| `gen.py` | ボール一覧から上位 Verilog・コア・`.asf`・`.sdc`・`params.txt` を `build/` に出す |
| `build.sh` | WSL で Supra を動かしてビットストリームを作り、配置配線の結果を検査する |
| `checkpnr.py` | 配置配線の後の検査（`build.sh` が呼ぶ）：実際のボール配置と一覧の完全一致、自動配置が無いこと、未使用のボールが無いこと |
| `gatesim.py` | 任意（`build.sh --gate-sim`）：配置配線後のネットリストを Supra 自身のモデルで Icarus Verilog にかける |
| `test_pinfind.py` | シミュレーションとツールのテスト（§8） |
| `testdata/sim_edges.txt` | シミュレーションで出た波形（各ボールのエッジ）。Rust の復号器のテストが読む |
| `../../tools/pinfind/mklist.py` | Supra の pinmap → ボール一覧 |
| `../../tools/pinfind/pins2asf.py` | 結果 → 本番用 `.asf` |
| `../../common/pinfind/` | 復号器（Rust、`no_std`、ホストでテスト）。`examples/la_csv.rs` はロジアナの CSV を読む |
| `../../mcu/netgd/src/pinfind.rs` | ファーム側（feature `pinfind`） |

## 3. 手順

### 3-0 前提

§0 のすべて。

### 3-1 ボール一覧を作る（Supra のある WSL で）

```sh
python3 tools/pinfind/mklist.py "$SUPRA_HOME/etc/AG16KDDF256.pinmap" -o fpga/pinfind/build/balls.txt
#   --exclude PIN_xx,PIN_yy  … 出力にしないボール（例：方法 1 で分かった 74HC04 の出力）。入力のまま設計に残る（hold）
#   --max-drive N            … 最初の N 本だけ走査（段階 1）。残りは hold
#   --no-hold-ddr            … DDR のボール F10・M10・L11・M11 を設計から外す（Supra が拒んだときだけ）
#   --leave-out PIN_xx,...   … hold のボールを設計から外す（Supra が拒んだときだけ）
```

- pinmap はリポジトリにコピーしない。一覧に書くのはボール名だけ（pinmap の 2 列目や機能名は写さない）。`fpga/pinfind/build/` は `.gitignore` 済み。
- **走査する（drive）のは、pinmap で 2 語の行（特別な機能の無いユーザー I/O）だけ。**
- **hold（入力として設計に入れ、決して駆動しない）**：`--exclude`・`--max-drive` の残り、**G1（DCLK）・G2（DATA0）・C1（ASDO）・D2（NCSO）**（構成後はユーザー I/O になる。
  `ag16k-config.md` C9）、pinmap で特別な機能の付いたほかのボール、DDR のボール（F10・M10・L11・M11）。
- 設計に入れないのは、専用のコンフィグのボール（nCONFIG H3・nSTATUS F3・CONF_DONE F16・MSEL1 F15）、JTAG（H1・H2・J1・J2）、NC（H4）だけ。
- **ボールの番号はこの一覧の drive の行の順番**。ビットストリームを作った一覧と、結果を読むときの一覧は同じファイルを使う（`pins2asf.py` が SHA-256 を記録する）。

### 3-2 設計を生成する（Windows、Amaranth の venv）

```powershell
cd fpga\pinfind
H:\OpenGDEmu\fpga\amaranth\amaranth_env\Scripts\python.exe gen.py --balls build\balls.txt            # 段階 1・2
H:\OpenGDEmu\fpga\amaranth\amaranth_env\Scripts\python.exe gen.py --balls build\balls.txt --stage0   # 段階 0
#   --clock osc|ring|ball:PIN_G1   クロック源（§1）
#   --probes                       段階 3：クロックボールを数える
#   --drive od|pp                  od = Low だけ出す（既定）、pp = High も出す（§5 では使わない）
#   --prescale N  --frames 2（1〜8）  --max-loops 1（1〜8）  --listen 16
#   --io-attrs                     駆動電流最小・弱プルアップ・未使用ピンを入力に、を .asf に足す（Quartus の書式。Supra が受け付けるかは未確認）
```

- **既定値が最初の実行の設定**（R143）：オープンドレイン、聞いてから出す、1 本あたり 2 フレーム、1 周、クロックプローブなし、T ≥ 20 µs。
- `gen.py` が断る組み合わせ：`--drive pp` と `--no-skip-low` の併用（衝突への保護が 2 つとも外れる）、`--max-loops` が 1〜8 の外、`--frames` が 1〜8 の外、`--stage0` とクロック・出力関係の指定の併用。
- `--no-probes` は昔のコマンドとの互換のために残してある（今は既定）。

### 3-3 ビットストリームを作る（WSL、Supra）

```sh
SUPRA_HOME=$HOME/supra-linux/x/Supra-2026.03.b0 fpga/pinfind/build.sh
#   → fpga/pinfind/output/pinfind_ag16k.rbf・pinfind_ag16k.params・pinfind_ag16k.txt
#   --placement FILE --placement-regex RE   実際の配置が書かれた Supra の報告と、その行の読み方（下）
#   --unused-ok PIN_xx,...                  使わないことを人が確認したボール（報告に残る）
#   --gate-sim [--gate-sim-ms 300]          配置配線後のネットリストのゲートレベル検査（Icarus Verilog が要る）
#   --gate-sim-done                         さらに「全パッドを駆動し、最後の 10 % は変化なし」（1 周で止まった）を要求（速い版専用）
#   --supra-retiming                        af_map の RETIMING 段（`abc -dff`）を Supra の既定どおり残す（試験専用、下の注意）
```

**af_map の RETIMING 段は切る**（2026-11-22、ゲートシミュレーションで発見）：Supra 2026.03.b0 の `af_map.tcl` は既定で
`dfflegalize` の前に `abc -markgroups -dff -D 100`（Yosys 0.61）をかける。この `abc -dff` はフリップフロップを
クロック・イネーブル・同期リセットの組で束ねて戻すが、**どちらが優先か**を落とす：「イネーブルが立っている時だけ効く同期リセット」
（`$sdffce`）が「いつでも効く同期リセット」（`$sdffe`）になって戻る。PinFindCore の `ball` は NEXT の中で `ball == n-1` のとき 0 に
していたので、マップ後は LISTEN 中に `ball` が 131 になった次のクロックで 0 に戻り、131 番は駆動されず、DONE に行かず回り続けた。
最小の再現は 8 ビットのカウンタ `if (en) c <= (c == 131) ? 0 : c + 1;`（en=0 で 131 に留まるはずが 0 になる）。
`build.sh` は `af_setup` が作る空の `PinFind.pre_map.asf` に `set RETIMING "None"` を書き（`af_map.tcl` は既定値を決めた後でこれを読む）、
`map.log` で `DFFLEGALIZE` より前に ABC が走っていないことを確かめる（走っていれば止まる）。あわせてコア（`hw/pinfind.py`）も、
1 周の終わりを FSM の外に「`state == NEXT` を含む条件」で書き直し、どちらの解釈でも同じレジスタになるようにした（RETIMING を残しても正しい）。
`fpga/agm/build.sh`（本体のコア）も同じ `af_map` を通るので同じ危険がある（未対処）。

`build.sh` は最初に古い `output/` の 3 ファイルを消す（失敗したビルドの後に古いビットストリームが残らない）。止まる条件：

| 検査 | 内容 |
|------|------|
| ファイル | `.rbf` が 552,352 B でない／先頭が `69 7e 3c 5a` でない |
| エラー | 「ピン位置が未割り当て」（`.asf` を読む前に必ず出る種類）以外の Error |
| **自動配置**（R143 の 1） | `run.out` に `placed at pin` が 1 行でもある（位置指定が効かなかった I/O を `af` が勝手に空きボールに置いた） |
| ピン数 | Supra の報告のピン数が一覧から作ったポート数と違う |
| **実際の配置**（`checkpnr.py`） | Supra の出力から「ポート → 実際のボール」を読み、`balls.txt` から `portmap.py` の規則で決まる配置と**完全一致**しない（hold も含む）。**読み取れなければ止まる** |
| **未使用のボール**（`checkpnr.py`） | `$SUPRA_HOME` の pinmap（その場で読むだけ）でユーザー I/O になりうるボールが 1 本でも設計に入っていない（`--unused-ok` で名指ししたものを除く） |
| **RETIMING 段**（上の注意） | `map.log` で `DFFLEGALIZE` より前に ABC が走った（`--supra-retiming` なしで）／`pre_map.asf` の仕掛けが Supra に無い |
| ゲートレベル（任意） | `--gate-sim`：配置配線後のネットリストを Supra のモデルでシミュレーションし、「同時に Low のパッドは 1 本以下（段階 0 は 0 本）」「X にならない」「指定時間内に 1 回は駆動する」。`--gate-sim-done` で「全パッドを駆動して止まった」も |
| そのほか | `osc` で `alta_osc` が、`ring` で LUT セルがマップ後のネットリストに無い／`fixio.py` でまとめた入出力の数が drive の本数と違う |

**実際の配置の読み取りについて**（Supra 2026.03.b0 の最初のビルドで確認、2026-11-22）：
- ボール名で「ポート → ボール」を書いた Supra の出力は **`alta_db/io.asf`** だけ（af_run が `place_pseudo -place_io` の後、ロジックの配置の前に書く I/O 配置のデータベース。
  ポートは逆順、括弧なし、各ポートに IO_STANDARD の行）。`.asf` の書式だが**要求ではなく Supra の結果**：`.asf` を読む前の段階のログは「Place virtual pin HOLD[0] at PIN_T16」と言い、
  io.asf は G1 と言う。要求から `P[5]`（C4）の割り当てを外した実験では、Supra は `Warn: IO P[5] is not assigned, placed at pin DDR_DM1.` と出して
  （`build.sh` はここで止まる）、io.asf にも `P[5] DDR_DM1` と Supra の選んだ場所が出た。ネットリストの座標（45, 29, 1）もほかのビルドのどのボールとも違った。
  `checkpnr.py` は io.asf を既定で読む（`--placement` は不要）。PIN_ でない場所（DDR_・NC_PIN_）もそのまま不一致として報告する。
- 配置配線後のネットリスト `PinFind_routed.v` にボール名は無く、各パッドのセルにダイ上の座標（`coord_x/y/z`）があるだけ。座標 → ボールの表は Supra の暗号化されたアーキテクチャのファイルの中で読めない。
  そこで `checkpnr.py` は、全ポートにパッドのセルがちょうど 1 つ、2 つのポートが同じ座標に無い、ほかのパッドのセルが無い、を確かめ、座標を `output/pinfind_ag16k.sites` に残す。
  `--sites-ref 前の.sites` を付けると、同じボールが同じ座標にあることを別のビルドと比べる（`osc`・`ring`・本番の NetGD のビルドの 66 本で一致を確認）。
- 読み取れなければ（Supra の版が変わったなど）「配置は確認されていない。このビットストリームを使うな」で止まる。そのときは人が作業フォルダの中（`run.out`・`alta_db/`・`PinFind_routed.v`）から
  **各 I/O が実際に置かれたボール**を書いた報告を探し、その行に合う正規表現（`(?P<port>…)` と `(?P<ball>…)` の名前付きグループ）を付けて `build.sh` をもう一度走らせる。
例：`--placement build/supra/alta_db/io.rpt --placement-regex '^\s*(?P<port>\S+)\s+.*\b(?P<ball>PIN_[A-T][0-9]+)\b'`（形式は仮の例）。
**`.asf`・`.qsf`・Tcl や、制約をなぞっただけの報告を指定してはいけない**（要求を「確認」してしまう）。`set_location_assignment` を含む行は読まない。
**io.asf も Supra の自己申告**であることは変わらない。シリコン上でそのボールに出ることは、段階 0〜1 の実機（ロジアナ）でしか確かめられない。

**未使用のボールについて**：Supra が未使用ピンをどう扱うか（入力か、GND を出す出力か）は分からない。だから確認する代わりに、ユーザー I/O になりうるボールを**全部設計に入れる**（drive か hold）。
Supra がどれかの割り当てを拒んだら（Error）、そのボールは `mklist.py --leave-out`（DDR なら `--no-hold-ddr`）で外し、
Supra の報告でそのボールがユーザー I/O でないこと（または入力のまま残ること）を人が確かめてから `build.sh --unused-ok PIN_xx` を付ける。報告（`.txt`）にその名前が残る。

Supra が受け付けなかった場合の切り分け：

- `alta_osc` が通らない → `--clock ring`、次に `--clock ball:PIN_G1`
- 走査するボールへの割り当てで Error → そのボールを `mklist.py --exclude` して一覧から作り直す（番号がずれるので、必ず一覧から）
- hold のボールへの割り当てで Error → 上の「未使用のボールについて」
- `--io-attrs` の行で Error → 外す

`build.sh` が確かめないこと（人の仕事）：Supra の報告どおりにシリコンが動くこと、電気的なふるまい、基板（PS のピン・電源・プルアップ）、実機でのすべて。

### 3-4 ファームを作る・書く

段階ごとのビットストリームとファームを一度に作り、ビットストリームの SHA-256 を表示するスクリプト：
`powershell -File tools\pinfind\mkstage.ps1 -Stage 0`（段階 1 は `-Stage 1` = `mklist.py --max-drive 4` + 既定の設計、段階 2 は `-Stage 2` = 全ボール）。
mklist → gen.py → `flock $HOME/supra.lock` つきの `build.sh` → `cargo build`（既定の target ディレクトリ）の順に走らせ、`tools/pinfind/out/netgd-pinfind-stage<N>-<sha8>.elf` と同名の `.txt`（SHA-256・大きさ）を出す（`out/` は管理外）。
ファームは起動直後に `PF STAGE 0 (no outputs driven) bitstream sha256=…`（段階 0）か `PF STAGE 1-3 (DRIVES balls; NOT stage 0) …`（それ以外）を RTT に出す。

```sh
cd mcu/netgd
cargo build --release --no-default-features --features pinfind       # ログは残す（DEFMT_LOG=off にしない）
```

- `fpga/pinfind/output/pinfind_ag16k.rbf` があれば LZMA で埋め込み、同じ場所の `.params` を `build.rs` が読む。
  `.params` が無い・`.rbf` と大きさや cksum が合わない・`max_loops` が 1〜8 の外ならビルドが止まる。ビルド時の警告に `pinfind: clock …, toggle DCLK …` か `STAGE 0` と出る。
- `.rbf` が無ければ「ビットストリーム無し」のファームになる（起動すると RTT に理由を出して止まる。LED は使わない）。
- 書き込みは SWD（`probe-rs`、チップ名 `ATSAM3U2E`）。**純正を消す工程（ERASE）を含むので、§0 が済んでから**。手順は P6 で確定させる。形だけ書くと：
  `probe-rs download --chip ATSAM3U2E target/thumbv7m-none-eabi/release/netgd`

### 3-5 走らせて記録する

```sh
probe-rs attach --chip ATSAM3U2E target/thumbv7m-none-eabi/release/netgd | tee rtt.log
```

ファームの流れ（`mcu/netgd/src/pinfind.rs`）：

1. 起動直後：PS の 5 本以外の全 PIO を入力（プルアップ）に。`init` は SMC・UART・PCK0・HSMCI のピンを設定しない（R143 の 10）。LED（PB18）も出力にしない。PS の 5 本も入力（PA23・PA24・PA25 は駆動しない）。
2. **事前確認**（どの線も動かさない）：**GPBR3 の印**、ビットストリームの有無、全展開、長さ、CRC。電源を入れてから一度でも走った印があれば、FPGA を入れずに止まる（リセットで 2 回目が走らない）。
   このとき PA25 は駆動しないので、前回の構成が残っていれば FPGA は動いたまま。CONF_DONE が High ならログで知らせる：**すぐ電源を切る**。
   消し方：**電源を切る**（GPBR はバックアップ領域の電源が落ちると消える）か、SWD で `probe-rs write --chip ATSAM3U2E b32 0x400E129C 0`。
   書き込み直後の `probe-rs` のリセットは 1 回目として数えない（印を付けるのは 3 の後）。
3. **10 秒待つ**（PS の線はすべて入力のまま。ロジアナやプルアップを付けるのはこの間。§5）。
4. 印を付け、SysTick の割り込みで動く**独立の期限**を構える（構成 30 秒、走査 120 秒 + 3 秒、段階 0 は 20 秒 + 3 秒。過ぎたら割り込みの中で PA23・PA24 を入力に戻し、nCONFIG を Low）。
5. 構成前の全 PIO の値を記録 → **ハンドシェイク**：PA25 を Low に引き（オープンドレイン。High は駆動しない）、PB2 が Low になるのを確認、PA25 を離して PB2 が High に戻るのを確認。どちらも時間制限つき。失敗すると PA23・PA24 は一度も出力にならない。
   次に PA23・PA24 を入力のまま複数回読み、どちらかが一度でも Low なら出力にせず止まる（`PF data-line check FAILED`。PA25 は Low のまま）。
   通ったら PA23・PA24 を出力にして **1 回だけ**ビットストリームを流す。失敗・CONF_DONE が早い（`early_done`）・流した量が足りない → PA23・PA24 を入力に戻し、nCONFIG を Low にして止まる（やり直さない）。
   PB2 が PA25 に反応しなかったとき（PA25 が nCONFIG だと示せていない）だけは、PA25 も入力に戻す。
6. 成功したら PA23 と（`ball:PIN_G1` でなければ）PA24 を入力 + プルアップに戻し、構成後の全 PIO の値と比べて違いを `PF diff` で出す。
7. 段階 0：20 秒そのまま保持し（電流を読む）、変化があれば `PF diff` を出し、nCONFIG を Low にして終わる。
8. 段階 1〜3：聞く。新しい組み合わせは `PF new pin=PB9 idx=37`、10 秒ごとに `PF hit …`（累計）・`PF act …`・`PF clk …`・`PF unit=…`（`ball:` のクロックでは走査中は出さず、最後にだけ出す）。
   **最後のフレームから 30 秒・開始から 120 秒・500 ms 以上 Low のままの線（止まった走査）・期限**のどれかで終わり、nCONFIG を Low にして `PF end` を出して止まる。
   FPGA の側も `max_loops` 周（既定 1）で止まる。
9. パニック・HardFault・未処理の割り込みでは、まず PA23・PA24 を入力に戻し、ハンドシェイクを始めていれば nCONFIG を Low にする。終わったら電源を切る。

### 3-6 `.asf` にする

```sh
python3 tools/pinfind/pins2asf.py --balls fpga/pinfind/build/balls.txt --mcu rtt.log \
    --la la.txt --la-map la_map.csv --clock-auto -o fpga/agm/pins/gdemu60.asf
SUPRA_HOME=... fpga/agm/build.sh --pins fpga/agm/pins/gdemu60.asf
```

- MCU のピン → コアのポートの対応は `pins2asf.py` の表（SMC の 30 本は SAM3U のピン配置で決まる。IRQn = PA22・RSTn = PA30 は v5.5 の基板の選択で、6.0 では仮定）。違えば `--mcu-map` で直す。
- 1 本でも欠けると、出力に `NETGD-PROVISIONAL-PINS` の印が付き（`build.sh` は本番名で出さない）、終了コード 2。
- 同じポートに 2 つのボール、同じボールに 2 つのポートはエラー。8 割以上を占める番号が無いピンもエラー。
- 最後は方法 4（本番ビットストリームで FPGA[12] が `0xC0DE` と読めるか）で確かめる。

## 4. 出力の読み方（RTT）

| 行 | 意味 |
|----|------|
| `PF pin-finder: stage0=… toggle_dclk=… …` | このファームに入っているビットストリームの設定 |
| `PF stop: …` | 止まった理由（GPBR の印、ビットストリーム無し、構成の失敗・早すぎる CONF_DONE・量の不足）。ログに nCONFIG の状態（Low で保持、またはすべて入力）が出る。AG16K で CONF_DONE が毎回ストリームの終わりより早く上がるなら、それが正常かどうかは未確認：判断が要る（安全側に倒して止めている） |
| `PF FPGA configured (… ms, … bytes …)` | 構成できた。PA23（と PA24）を入力に戻した |
| `PF diff across-configuration port=B changed=0x… before=… after=…` | 構成の前後で MCU のピンの値が変わった（段階 0 では起きてはいけない） |
| `PF new pin=PB9 idx=37 unit=1536` | PB9 に初めてボール 37 が来た |
| `PF hit pin=PB9 idx=37 n=12` | 累計（`pins2asf.py` は同じ組の最大値を使う） |
| `PF act pin=PA21 edges=1234` | 変化はあったがフレームにならない（クロック、反転経路、T が短すぎる） |
| `PF clk c=3 cnt=96 unit=1536 div=10 win=6 hz=48000000` | クロックプローブ 3（`--probes` のときの一覧の `clock 3` のボール）が約 48 MHz |
| `PF unit=1536` | T が CPU 1,536 サイクル（= 16 µs） |
| `PF stuck-low pin=… ` | 走査の途中で線が Low のまま止まった。FPGA を止めた |
| `PF end (理由, … s)` | 終了。FPGA は nCONFIG Low で止めてある |

## 5. GD-ROM コネクタ側（DC_* の 34 本）

MCU からは見えないので、ロジックアナライザで読む（`g1-ag16k-port.md` §2-4 のとおり）。MCU 側の段階 2 が済んでから、別の回で行う。

1. 基板は Dreamcast から外し、§0 の方法で給電する。**Dreamcast に付けたまま走らせない**（§6）。SD は抜く。
2. ビットストリームは**既定のまま**（オープンドレイン、聞いてから出す）。`--drive pp` と `--no-skip-low` は使わない（`gen.py` は併用を断る）。
3. CN1 側にはプルアップが無い（Dreamcast 側にある）ので、**読む線ごとに外付けのプルアップ**（例：10 kΩ を基板の 3.3 V へ）を付ける。
   ロジアナの入力にプルダウンがあっても、プルアップのほうが強ければ LISTEN で Low と読まれず、飛ばされない。
4. **ロジアナとプルアップは、基板に電源を入れてから付け、電源を切る前に外す**。ファームは起動後 10 秒、FPGA を構成前のまま待つ（`PF waiting 10 s`）ので、その間に付ける。
   外すのは `PF end`（FPGA は nCONFIG Low）の後、電源を切る前。電源の入っていない基板に、電源の入ったロジアナやプルアップをつながない。
   プローブを当てるときに隣のピンと短絡させないこと（クリップの付いたハーネスを先に用意しておく）。
5. ロジアナは 3.3 V の閾値、入力は高インピーダンス。チャンネルが足りなければ何回かに分けて当て直す（1 回 = 1 周。毎回、電源を切って GPBR の印を消すところから）。
6. サンプリングは T の 4 倍以上（T = 20 µs なら 200 kHz 以上。余裕を見て 1 MHz 以上）。
7. `sigrok-cli … -O csv > cap.csv` か PulseView の CSV 書き出し → `cargo run --example la_csv -- cap.csv > la.txt`（`common/pinfind` で）。
8. `la_map.csv` にチャンネル名とポートの対応（例：`D3,DC_DATA[3]`）を CN1 のピン配置から書き、`pins2asf.py --la la.txt --la-map la_map.csv`。

## 6. 電気的なリスクと、この設計での抑え方

基板を壊したら戻せないので、ここがいちばん大事。**どれも実機で確かめていない。**

| リスク | 起きうること | 抑え方 |
|--------|--------------|--------|
| FPGA のボールが、ほかの出力とつながっている（衝突） | 両方が逆のレベルを出し、大きな電流が流れる | ① 同時に出力にするボールは 1 本だけ ② 既定はオープンドレイン（Low しか出さない） ③ 出す前に LISTEN し、Low を一度でも読んだら飛ばす ④ 1 本あたりの駆動は 1 周に 1 回、既定 2 フレーム、既定 1 周（最大 8）、駆動の見張り ⑤ ファームの期限（120 秒、割り込みから独立に nCONFIG を Low） ⑥ `--io-attrs` で駆動電流を最小に（Supra が受け付けるかは未確認） |
| 配置の誤り（R143 の 1） | `af` が位置指定の効かなかった I/O を勝手に置く、出力の向き（`.oe` の極性）が逆で全ボールが同時に Low | `build.sh` が自動配置の警告で止まり、実際の配置を一覧と完全照合する（読めなければ止まる）。`.oe` の極性は `--gate-sim`（Supra のモデルが正しければ）と段階 0・1 で確かめる |
| High に強く引かれた線（3.3 V 直結、相手の High 出力）に Low を出す | LISTEN では検出できない（プルアップと区別がつかない） | 残るリスク。1 本につき既定 2 フレーム × 1 周、しかも Low の時間はフレームの約半分。段階 1 で数本だけ試し、電流を見る |
| FPGA のクロックが止まる（`ball:` のクロックで MCU が止まる） | フレームの途中の 1 本が Low のまま固定される | ファームが 500 ms 以上 Low のままの線を見つけたら止める（MCU の見える線だけ）。MCU 自体が止まると何もできない：デバッガで止めない。`ball:` は最後の手段 |
| 絶対時間の上限 | FPGA は自分のクロックの周波数を知らないので、1 本あたりの時間を秒では制限できない | ファームの期限（構成後 120 秒）が絶対の上限。FPGA 側の上限はクロック数（駆動の見張り）と周回数 |
| MCU 側の衝突 | MCU と FPGA が同じ線を逆に駆動する | MCU が出力にするのは nCONFIG と、構成中だけの DCLK・DATA0。構成が終わるとすぐ PA23（と PA24）を入力 + プルアップに戻す（`ball:PIN_G1` のときだけ PA24 を振り続ける。そのとき G1 は FPGA の入力）。ほかの PIO は入力 + プルアップ、SMC・UART・PCK0・HSMCI は設定しない、LED も出力にしない |
| FPGA が動いたままになる | リセット・パニック・ロード失敗のあとも走査が続く | ロード失敗は 1 回で nCONFIG Low・停止、パニック・HardFault・未処理割り込みでも nCONFIG Low、期限は SysTick の割り込み、GPBR の印で 2 回目を走らせない。MCU のリセット中は nCONFIG が High になりうる（そのあいだ FPGA は動く）。次の起動は GPBR の印で止まり、PA25 を駆動しないので FPGA は動いたまま：CONF_DONE が High ならログで知らせる。**電源を切る** |
| Dreamcast に付けたまま走らせる | CSn・RDn・WRn・RSTn・DMACKn・ADDR は Dreamcast が常に駆動している | **Dreamcast から外して走らせる。** 給電は §0 の予行のとおり |
| 構成されていないボールの扱い | Supra が未使用ボールをどう扱うか（入力か、Low 出力か）は不明 | ユーザー I/O になりうるボールを全部設計に入れる（G1・G2・C1・D2・DDR のボール・クロックボールも hold）。`build.sh` が pinmap と照らして確かめる。段階 0 で構成前後の MCU のピンの差分を見る |
| 内蔵 DDR | DDR につながるボールを触ると壊すおそれ | DDR のボールは入力（hold）にするだけで駆動しない。DDR を休止させる信号は出していない（本番と同じく未対応） |
| 電圧 | I/O バンクの電圧（VCCIO）が 3.3 V でない場合 | FPGA は Low（と pp では High）を出すだけで、MCU は入力。3.3 V の MCU に 2.5 V の High が来ても壊れない |
| 給電 | 基板の外での給電を間違えると、それ自体が壊す原因になる | §0-1 の 3 の予行。SWD ヘッダの 1 番から入れる案は、レギュレータへの逆流の心配があり勧めない |

## 7. 本設計で決めたことの理由

- **番号は 1 バイト + 反転の 1 バイト**：雑音やクロックの線がたまたまフレームの形になっても、反転が合う確率は 1/256。クロック計測つきは和も付ける。
- **パルス幅の比で 0/1**：オシレータの周波数が 1 桁わからなくても読めるように。T は SYNC の 9T 全体から測るので、エッジの揺れの影響が小さい。
- **全レジスタの初期値 0**：AGM のフローが 0 以外の初期値をどう扱うかに依存しない。最初の状態（START）で 256 ティック待ってから始める。
- **ボール一覧は生成物**：Supra の pinmap をリポジトリに入れないため。番号の対応を固定するため一覧に SHA-256 を付け、各ツールが記録する。
- **未使用ピンの扱いを確かめる代わりに、未使用を作らない**：Supra の既定は資料が無く、報告の形式も分からない。設計に全部入れれば、その問いが要らなくなる。
- **ファームの動作はビットストリームの `.params` から決める**：DCLK を振るか・段階 0 かを人が合わせ忘れないように。`.rbf` の cksum で組み合わせの取り違えも防ぐ。
- **SD に結果を書かない**：ファームに SD の書き込みが無い（`blockdev.rs` は読み出し専用）。RTT のログで足りる。

## 8. 確かめたこと（この PC で）

- `test_pinfind.py`（Amaranth 0.5.8 のシミュレーションとツール、15 件）：走査順と波形のティック単位の一致、prescale 0、Low のボールを飛ばす、
  `max_loops` で止まって以後出力しない、大きな番号、クロックプローブのカウント、`max_loops` の範囲（0・9 を断る）、駆動の見張り（上限で出力が切れて止まる／既定の上限では切れない）、
  `gen.py` の出力（全ポートにボールが 1 つずつ・`.asf` が `portmap.py` と一致・pinmap のユーザー I/O になりうるボールが全部入る・G1/G2/C1/D2/DDR が hold・
  DCLK を振るのは `ball:PIN_G1` だけ・既定値）、段階 0 の設計に出力が無いこと、危ない組み合わせを断ること、
  `checkpnr.py`（作り物の報告で：完全一致なら通る、入れ替わり・自動配置・配置が読めない・`.asf` を指定・未使用のボール・古い一覧で止まる、`--unused-ok`・`--placement-regex`）、
  `gatesim.py` のテストベンチ生成（作り物のネットリストの 2 通りのポートの書き方）。毎クロック「出力が 2 本以上にならない」「FRAME 以外で出力しない」を確かめている。
- `common/pinfind`（`cargo test`、16 件）：1 ティック 3〜1,000,000 単位、ドリフト、±1/4 ティックの揺れ、反転、クロック・乱数が誤ってフレームにならない、
  グリッチ、時刻の折り返し、途中で切れたフレーム、クロック部の往復、**Amaranth のシミュレーションの波形（`testdata/sim_edges.txt`）を復号して番号・T・周波数が合う**、ロジアナ CSV の読み込み。
- `tools/pinfind/tests`（`python -m unittest`、14 件）：作り物の pinmap で一覧、除外、hold の中身、pinmap の全ボールが設計に入ること、`--no-hold-ddr`・`--leave-out`、`pins2asf.py`。
- ファーム：`--no-default-features --features pinfind` がビルドできる（ビットストリーム無し、および中身が無意味な 552,352 B のファイルと `.params` を一時的に置いて
  `osc`・`ball:PIN_G1`・段階 0 の 3 通り。`.params` の cksum が合わないとビルドが止まることも確認）。通常のファームは、2E の 3 構成（`sam3u2e`・`sam3u2e,resident,sd-fast` の release-z・`board-v60`）がこの変更の前とロードされる中身でバイト一致。4E（ログあり）は defmt の文字列番号の 13 B だけ違い、`DEFMT_LOG=off` で作ると一致（ソースの位置が変わると defmt の番号の並びが変わるため）。
- `test_pinfind.py` の 15 件目：Supra 2026.03.b0 の形の作業フォルダ（`alta_db/io.asf`・座標つきの `PinFind_routed.v`、中身は作り物）で、通る／入れ替わり・NC_PIN・欠け・余分な I/O・同じ座標・ネットリスト無し・`--sites-ref` の不一致で止まる。
- **本物の Supra（2026.03.b0、WSL、2026-11-22）**：`mklist.py` → `gen.py` → `build.sh` が最後まで通った（`osc` 既定・`ring`）。`.rbf` 552,352 B、LZMA 約 14 KB、ピン 169/251、
  `placed at pin` なし、`checkpnr.py` が io.asf を読んで 169 本すべて一致、ネットリストの座標 169 個がすべて別、pinmap のユーザー I/O になりうるボールで設計に無いものは 0 本。
  hold（G1・G2・C1・D2・特別な機能・DDR の F10/M10/L11/M11・クロックボール）の割り当ても Supra は受け付けた（hold のパッドは `.oe(gnd)`）。
  `.asf` を読む前の「locatiion is not assigned」の Error は hold の 37 本ぶん出る（既知の種類で、`build.sh` は除外している）。
- `gatesim.py`（Icarus Verilog 12.0、WSL）：Supra のモデルから必要なモジュールだけを `--work` に取り出して使う（`alta_sim.v` の ADC は SystemVerilog 専用で Icarus が読めず、`etc/` に同じ写しがある）。
  本番の設定（prescale 10・start 256）は 1 周が実時間で約 1.2 秒分になり Icarus では何時間もかかるので、ゲートシミュレーション用に `gen.py --prescale 1 --start 8` で別に作ったもの（論理は同じでティックだけ短い）にかける。
  リングは遅延ゼロだと発振しないため、`--clock ring` ではモデルの LUT に 0.4 ns の遅延と、リングの最初の段だけに「LUT の出力が X なら 0」を足す（シミュレーション専用の上書き）。
  結果（2026-11-22、最初）：段階 0 と osc は合格（同時に Low は最大 1 本）。ただし配置配線後のネットリストは RTL と違い、1 周で止まらずに回り続け、最後のボール（131）を駆動しない。
  原因は af_map の RETIMING 段（§3-3 の注意）。マップ直後の `.vqm` でもう同じ（131 番で 0 に戻る）なので配置配線ではない。
  確かめ方：`.vqm` を正しい `dffeas`（Supra のシミュレーションモデルの `dffeas` は `ena`・`sclr` を無視する）と `cin` をつないだ
  `cycloneive_lcell_comb`（Supra のモデルは `cin_w` が未接続でキャリーが X）で動かし、RTL とパッドの変化を 1 行ずつ比べた
  （130 番までの 8,908 行は一致、131 番で分かれる）。Yosys の途中の段で書き出して比べると、RETIMING の `abc -dff` の前は正しく後は誤り。
  修正後（コアの書き直し＋RETIMING を切る）：osc の速い版は配置配線後も「132 本すべて駆動、1,436.5 µs で停止、同時に Low は最大 1 本」
  （`--gate-sim-done` 合格）、マップ後の `.vqm` と RTL のパッド変化 8,977 行が完全一致。コアの書き直しだけ（`--supra-retiming`）でも、
  RETIMING を切るだけ（古いコア）でも、それぞれ単独で同じ結果。段階 0 も合格。
  ring は最初、前の版の上書き（「LUT の入力に X があれば 0」）のせいで発振しなかった（`half` の LUT は未接続の入力 A・B・D が Z で、出力を常に 0 にされていた）。
  全スライスで「出力が X なら 0」にすると、今度は 31 段が同じ値から始まって LUT 遅延ごとに一斉に反転する偽の 1.25 GHz になり何も動かない。
  そこで「出力が X なら 0」はリングの最初の段（`u_clk|stage[0].inv`、ネットリストから探してテストベンチの defparam で指定）だけにした。
  これで ring の速い版も配置配線後に合格：132 本すべて駆動、1,781.2 µs で停止、同時に Low は最大 1 本（パッド変化 8,977 回は osc と同じ）。
- 確かめていないこと：Supra の報告どおりにシリコンが動くこと、`alta_osc` の周波数・極性、実機のすべて。

## 9. 設計メモ：逆向きの探し方（R143 の 9。未実装・要判断）

R143 が提案した別の方式。**実装していない。** 採るかどうかは後で決める。

- **しくみ**：FPGA は全ボールを入力にし、報告には既知の 1 本だけを使う。MCU が自分のピンを 1 本ずつ動かし（例：1 本ずつ Low にする）、
  FPGA は「いま変化したボール」の番号をその 1 本で MCU に送り返す。FPGA は未知のボールを 1 本も駆動しない。
- **良い点**：FPGA 側の衝突のリスク（§6 の 1 行目・3 行目）が無くなる。駆動するのは MCU のピンだけで、MCU の出力の相手は FPGA の入力か、何もつながっていないか。
- **報告に使う 1 本の候補**：G2（DATA0、MCU の PA23）。構成の後は MCU 側を入力にでき、配線が分かっている数少ない線。
- **未解決の問い**：
  1. **Supra が G2 へのユーザー信号（出力）の割り当てを許すか**（DCLK・DATA0 は「通常の I/O としても使える」とマニュアルにあるが、Supra のフローでは未確認）。許さなければ報告の線が無い。
  2. MCU のピンを動かすこと自体の安全性：MCU のピンの相手が FPGA 以外（CN1 側の部品、電源、ほかの IC の出力）だと、今度は MCU が衝突する。オープンドレイン風（出力は Low だけ、1 本ずつ、短時間）にしても「High に強く引かれた線」の問題は MCU 側に移るだけ。
  3. CN1 側の 34 本は MCU から動かせないので、この方式では見つからない（ロジアナの手順は別に残る）。
- **次の一歩（決めるなら）**：段階 0 の設計に G2 の出力だけを足した版で、Supra が受け付けるか（`build.sh` の検査が通るか）を見る。実機に流すのはその後の判断。
