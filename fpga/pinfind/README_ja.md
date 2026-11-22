# fpga/pinfind — GDEMU 6.0 のピン探索（方法 2）

`docs/design/g1-ag16k-port.md` §2-4 の「方法 2：ピン探索用ビットストリーム」の実装。
6.0 基板の FPGA（AGM AG16KDDF256）のどのボールが、MCU（ATSAM3U2E）のどのピン・GD-ROM コネクタ（CN1）のどのピンにつながっているかを、
FPGA 自身にボールの番号を名乗らせて調べる。結果は `fpga/agm` の本番ビルド用のピンファイル（`.asf`）に機械的に変換できる。

**実機では何も確かめていない。** この PC には Supra も WSL も無いので、ビットストリームも一度も作っていない。
確かめたのは、Amaranth のシミュレーション・ホストのテスト・ファームのビルドまで（§8）。

> **基板を壊すと元に戻せない。** 純正ファームは読み出せない場合があり（`tools/fwbackup`）、消したら戻せない。
> この手順は FPGA のボールを 1 本ずつ出力にする。§6 の電気的なリスクと対策を読んでから、2 枚目の基板があるときに進めることを勧める
> （`g1-ag16k-port.md` §0 の 5）。

## 1. しくみ

```
Supra の pinmap ──mklist.py──▶ build/balls.txt（ボール名の一覧。コミットしない）
                                   │
          hw/pinfind.py ──gen.py──▶ build/PinFind.v・PinFindCore.v・PinFind.asf・PinFind.sdc・params.txt
                                   │
                         build.sh（WSL + Supra）──▶ output/pinfind_ag16k.rbf（コミットしない）
                                   │
   mcu/netgd --features pinfind ◀──┘ （ビットストリームを埋め込む。board-v60 と同じ方式）
            │ SWD で書き込み、RTT でログを取る
            ▼
        rtt.log ──┐
   ロジアナ CSV ─la_csv─▶ la.txt ──┼──pins2asf.py──▶ fpga/agm/pins/gdemu60.asf
                                   ┘
```

FPGA 側（`hw/pinfind.py`）がボール `k` ごとにすること：

1. **LISTEN**：全出力を切ったまま `listen` ティック、ボール `k` を見る。**一度でも Low を読んだら、そのボールは飛ばす**
   （プルアップ以外の何かが Low に引いているか、動かしている。クロックバッファの出力、駆動中のバス、GND に直結したボールなど）。
2. **FRAME**：ボール `k` だけ出力を有効にして、番号 `k` のフレームを `frames` 回送る。
3. **NEXT**：出力を切って次のボールへ。最後のボールの次で 1 周。`max_loops` 周（既定 8）で止まり、以後は全出力オフのまま。

同時に出力になるボールは常に 1 本以下（シミュレーションで毎クロック確かめている）。

### フレームの形（形式 1。`common/pinfind/src/lib.rs` が正）

1 ティック T = FPGA のクロック 2^prescale 個。「アクティブ」= Low を出す、「アイドル」= High（オープンドレインでは手を離してプルアップに任せる）。

```
アイドル | SYNC: アクティブ 8T | アイドル 1T | ビット: アクティブ 1T（0）か 3T（1）+ アイドル 1T | … | アイドル 4T 以上
```

- 中身は MSB から：`idx`、`~idx`（番号とその反転）。クロック計測つきなら続けて `hdr`（`div_bits << 4 | win_bits`）、
  クロックボールごとに 16 bit のカウント、最後に全バイトの和（下位 8 bit）。
- 受け側は SYNC とその直後のアイドル（計 9T）から T を測り、以後もビットごとに追従する。比べるのはパルス幅どうしだけなので、
  **FPGA のクロックが数百 kHz でも数百 MHz でも、ゆっくり揺れても同じに読める**（`common/pinfind/tests/decode.rs`）。

### クロックボール（入力専用）の見つけ方

専用クロック入力（J4・K1・L1・G16・G15・H16・H15・F9・E9・T14・R14）は出力にできないので、走査では見つからない。
代わりに各クロックボールで小さなカウンタを回し、その最上位ビットの変化回数を一定のティック数で数えてフレームに載せる（クロックプローブ）。
受け側は自分で測った T から周波数に直す（`pinfind::clock_hz`）。48 MHz（MCU の PCK0 か PWM）と 11.2896 MHz（74HC04 経由の水晶）が
どのボールに来ているかが分かる。

### FPGA のクロック源（`--clock`）

どのクロックボールに何が来ているか分からない状態で動く必要がある。3 通りを用意した（`src/pf_clock.v`）。

| `--clock` | 中身 | 未確認の点 |
|-----------|------|-----------|
| `osc`（既定） | AGM の内蔵オシレータ `alta_osc`（`i_osc_enb`、`o_osc`）。Supra のプリミティブ集に名前がある（rodinia の写しで確認された、という調査結果。Supra のファイルはリポジトリに入れていない）。シミュレーションモデルは `i_osc_enb` が 0 のときだけ動くので、Low で有効として 0 を入れた | AG16KDDF256 で配置できるか、極性、周波数（UFM のモデルのコメントでは数 MHz らしいが公表値は無い） |
| `ring` | LUT（`cycloneive_lcell_comb`）31 段のリングオシレータ ＋ 2 分周。Yosys に消されないよう LUT セルを直接置く | `af` が組み合わせループを配置配線するか。周波数（配線しだい。5〜200 MHz と見ている） |
| `ball:PIN_G1` | ボールをクロック入力に使う。G1 = DCLK は MCU が PS で使うピン（PA24）で、`pinfind` ファームは構成後も PA24 を約 1 MHz で振り続ける | DCLK を構成後にユーザー入力として使えるか（DDF256 マニュアルは「SPI の 4 本は通常の I/O としても使える」と書く）。Supra が G1 へのユーザー信号の割り当てを許すか |

既定は `osc`、だめなら `ring`、それもだめなら `ball:PIN_G1`。方法 1（テスター）で 48 MHz か 11.2896 MHz のボールが分かれば
`ball:PIN_xx` でそれを使ってもよい（そのボールはクロックプローブから外れるので `--no-probes` を付ける）。
`--clkctrl`（`osc`・`ring` では既定でオン）はクロックを明示的な `cycloneive_clkctrl`（`ena` = 1）経由でグローバル網に載せる。
「Yosys が enable を 0 に固定した clkctrl を勝手に入れて af が失敗する」という二次情報への対策。

ティック（`--prescale`）の既定：`osc` 8、`ring` 11、`ball` 5。MCU は約 2 MHz で全ピンを見ている（推定。§8）ので、
**T は 5 µs 以上ほしい**。ログの `PF unit=<サイクル数>` から FPGA のクロックが分かる：f = 2^prescale × 96,000,000 / unit。
`PF act`（変化はあるがフレームにならない）ばかりなら、prescale を上げて作り直す。

## 2. ファイル

| ファイル | 中身 |
|----------|------|
| `hw/pinfind.py` | 本体（Amaranth）：LISTEN / FRAME / NEXT の順序回路、フレームの符号器、クロックプローブ |
| `src/pf_clock.v` | クロック源（`alta_osc` / リング / ボール）と `cycloneive_clkctrl` |
| `gen.py` | ボール一覧から上位 Verilog・コア・`.asf`・`.sdc`・`params.txt` を `build/` に出す |
| `build.sh` | WSL で Supra を動かしてビットストリームを作る（`fpga/agm/build.sh` と同じ決まり：`unshare -rn`、Supra のツリーに書かない） |
| `test_pinfind.py` | シミュレーション（下の §8） |
| `testdata/sim_edges.txt` | シミュレーションで出た波形（各ボールのエッジ）。Rust の復号器のテストが読む |
| `../../tools/pinfind/mklist.py` | Supra の pinmap → ボール一覧 |
| `../../tools/pinfind/pins2asf.py` | 結果 → 本番用 `.asf` |
| `../../common/pinfind/` | 復号器（Rust、`no_std`、ホストでテスト）。`examples/la_csv.rs` はロジアナの CSV を読む |
| `../../mcu/netgd/src/pinfind.rs` | ファーム側（feature `pinfind`） |

## 3. 手順

### 3-0 前提

- **純正ファームのバックアップ**：`tools/fwbackup/README_ja.md` のとおり `check` → `UNLOCKED_2E` なら `dump`。
  `LOCKED` なら純正は戻せない。そのまま進めるかはユーザーの判断（2 枚目の基板を勧める）。
- PS のピン 5 本（PA23 DATA0・PA24 DCLK・PA25 nCONFIG・PB1 CONF_DONE・PB2 nSTATUS）は **v5.5 の値を仮に使っている**（`cyclone::pinmap`、未確認）。
  違っていればビットストリームが入らず、LED が点滅するだけ（壊れはしない）。先に導通確認できれば確実。
- §6 の「基板の外で給電する」準備。

### 3-1 ボール一覧を作る（Supra のある WSL で）

```sh
python3 tools/pinfind/mklist.py "$SUPRA_HOME/etc/AG16KDDF256.pinmap" -o fpga/pinfind/build/balls.txt
#   --exclude PIN_xx,PIN_yy  … 出力にしないボール（例：方法 1 で分かった 74HC04 の出力）。入力のまま設計に残る（hold）
#   --max-drive N            … 最初の N 本だけ走査（試験用）
```

- pinmap はリポジトリにコピーしない。一覧に書くのはボール名だけ（pinmap の 2 列目や機能名は写さない）。`fpga/pinfind/build/` は `.gitignore` 済み。
- コンフィグ（G1・G2・H3・F3・F16・F15）、JTAG（H1・H2・J1・J2）、NC（H4）、DDR（F10・M10・L11・M11）は pinmap に何と書いてあっても載せない。
  pinmap で 3 語以上の行（特別な機能のボール）も使わない。
- **ボールの番号はこの一覧の行の順番**。ビットストリームを作った一覧と、結果を読むときの一覧は同じファイルを使う（`pins2asf.py` が SHA-256 を記録する）。

### 3-2 設計を生成する（Windows、Amaranth の venv）

```powershell
cd fpga\pinfind
..\amaranth\amaranth_env\Scripts\python.exe gen.py --balls build\balls.txt
#   --clock osc|ring|ball:PIN_G1   クロック源（§1）
#   --drive od|pp                  od = Low だけ出す（既定）、pp = High も出す（ロジアナ用。§5）
#   --prescale N  --frames 3  --max-loops 8  --listen 16
#   --io-attrs                     駆動電流最小・弱プルアップ・未使用ピンを入力に、を .asf に足す（Quartus の書式。Supra が受け付けるかは未確認）
```

### 3-3 ビットストリームを作る（WSL、Supra）

```sh
SUPRA_HOME=$HOME/supra-linux/x/Supra-2026.03.b0 fpga/pinfind/build.sh
#   → fpga/pinfind/output/pinfind_ag16k.rbf と pinfind_ag16k.txt
```

`build.sh` が止まる条件：`.rbf` が 552,352 B でない／先頭が `69 7e 3c 5a` でない／ピン数が一覧（drive + hold + probe (+ CLKIN)）と合わない／
`osc` で `alta_osc` が、`ring` で LUT セルがマップ後のネットリストに無い／「ピン位置が未割り当て」以外の Error。
走査するボールは入出力（inout）なので、`fpga/agm/fixio.py --expect <drive 本数>` で 1 パッドにまとめている（R99 §4-1 の問題）。

Supra が受け付けなかった場合の切り分け：

- `alta_osc` が通らない → `--clock ring`、次に `--clock ball:PIN_G1`
- あるボールへの割り当てで Error → そのボールを `mklist.py --exclude` して一覧から作り直す（番号がずれるので、必ず一覧から）
- `--io-attrs` の行で Error → 外す（そのときの保護は §6 のオープンドレイン・LISTEN・周回数の上限だけになる）

### 3-4 ファームを作る・書く

```sh
cd mcu/netgd
cargo build --release --no-default-features --features pinfind       # ログは残す（DEFMT_LOG=off にしない）
```

- `fpga/pinfind/output/pinfind_ag16k.rbf` があれば LZMA で埋め込む。無ければ「ビットストリーム無し」（LED 5 回点滅）のファームになる。
- 書き込みは SWD（`probe-rs`、チップ名 `ATSAM3U2E`）。**純正を消す工程（ERASE）を含むので、3-0 が済んでから**。手順は P6 で確定させる。形だけ書くと：
  `probe-rs download --chip ATSAM3U2E target/thumbv7m-none-eabi/release/netgd`

### 3-5 走らせて記録する

```sh
probe-rs attach --chip ATSAM3U2E target/thumbv7m-none-eabi/release/netgd | tee rtt.log
```

- ファームは FPGA を入れたあと、PS の 5 本以外の全 PIO を入力（プルアップ）にして聞く。新しい組み合わせは `PF new pin=PB9 idx=37`、
  10 秒ごとに `PF hit …`（累計）、`PF act …`、`PF clk …`、`PF unit=…` を出す。
- **最後のフレームから 30 秒、または開始から 300 秒で終わる**：nCONFIG を Low にして FPGA を構成前の状態（ユーザー I/O オフ）に戻し、`PF end` を出して止まる。
  FPGA の側も `max_loops` 周で止まる。終わったら電源を切る。

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
| `PF new pin=PB9 idx=37 unit=1536` | PB9 に初めてボール 37 が来た |
| `PF hit pin=PB9 idx=37 n=12` | 累計（`pins2asf.py` は同じ組の最大値を使う） |
| `PF act pin=PA21 edges=1234` | 変化はあったがフレームにならない（クロック、反転経路、T が短すぎる） |
| `PF clk c=3 cnt=96 unit=1536 div=10 win=6 hz=48000000` | クロックプローブ 3（一覧の `clock 3` のボール）が約 48 MHz |
| `PF unit=1536` | T が CPU 1,536 サイクル（= 16 µs） |
| `PF end` | 終了。FPGA は nCONFIG Low で止めてある |

## 5. GD-ROM コネクタ側（DC_* の 34 本）

MCU からは見えないので、ロジックアナライザで読む（`g1-ag16k-port.md` §2-4 のとおり）。

1. 基板を Dreamcast から外し、§6 の方法で給電する。**Dreamcast に付けたまま走らせない**（§6）。
2. ロジアナ（3.3 V の閾値、入力は高インピーダンス）を CN1 のピンに当てる。チャンネルが足りなければ何回かに分けて当て直す（1 回 = 1 周以上）。
3. CN1 側にはプルアップが無い（Dreamcast 側にある）ので、**`--drive pp` か `--io-attrs`（弱プルアップ）で作ったビットストリーム**を使う。
   既定のオープンドレインのままだと High にならず、何も見えない。
   ロジアナの入力にプルダウンがある場合、LISTEN で Low と読まれて飛ばされることがある。そのときは `--no-skip-low` の版を使う
   （CN1 には何もつながっていない前提。MCU 側の確認とは別の回で）。
4. サンプリングは T の 4 倍以上（T = 16 µs なら 250 kHz 以上。余裕を見て 1 MHz 以上）。
5. `sigrok-cli … -O csv > cap.csv` か PulseView の CSV 書き出し → `cargo run --example la_csv -- cap.csv > la.txt`（`common/pinfind` で）。
6. `la_map.csv` にチャンネル名とポートの対応（例：`D3,DC_DATA[3]`）を CN1 のピン配置から書き、`pins2asf.py --la la.txt --la-map la_map.csv`。

## 6. 電気的なリスクと、この設計での抑え方

基板を壊したら戻せないので、ここがいちばん大事。**どれも実機で確かめていない。**

| リスク | 起きうること | 抑え方 |
|--------|--------------|--------|
| FPGA のボールが、ほかの出力とつながっている（衝突） | 両方が逆のレベルを出し、大きな電流が流れる | ① 同時に出力にするボールは 1 本だけ ② 既定はオープンドレイン（Low しか出さない。High 側でぶつからない） ③ 出す前に LISTEN し、Low を一度でも読んだら飛ばす（Low に駆動・発振している相手とは戦わない） ④ 1 本あたりの駆動は 1 周に 1 回・数十 ms、周回数は `max_loops`（既定 8）で打ち切り、ファームも 300 秒で nCONFIG を Low にする ⑤ `--io-attrs` で駆動電流を最小に（Supra が受け付けるかは未確認） |
| High に強く引かれた線（3.3 V 直結、相手の High 出力）に Low を出す | ③では検出できない（プルアップと区別がつかない） | 残るリスク。1 本につき 1 周あたり数十 ms × 最大 8 周、しかも Low の時間はフレームの約半分。設計者が未使用ボールを VCC に直結することは普通ないが、ゼロではない |
| MCU 側の衝突 | MCU と FPGA が同じ線を逆に駆動する | MCU は PS の 3 本（nCONFIG・DCLK・DATA0）以外を一切駆動しない。全 PIO をリセット直後と同じ「入力 + プルアップ」にしてから FPGA を入れる（`embassy_atsam3::init` が立てた SMC・UART・PCK0・HSMCI も戻す） |
| Dreamcast に付けたまま走らせる | CSn・RDn・WRn・RSTn・DMACKn・ADDR は Dreamcast が常に High/Low を出している。High の線にオープンドレインで Low を出すと衝突する（LISTEN は High を「プルアップ」と区別できない） | **Dreamcast から外して走らせる。** 給電方法は未確認（下） |
| 構成されていないボールの扱い | Supra が未使用ボールをどう扱うか（入力か、Low 出力か）は不明 | 一覧の全ボールを設計に入れる（走査しないものも hold として入力で残す）。`build.sh` は Supra の報告のピン数が一覧と一致することを確かめる。残るのはコンフィグ・JTAG・DDR・pinmap で特別扱いのボールだけ |
| 内蔵 DDR | DDR につながるボールを触ると壊すおそれ | pinmap に出てくる DDR 系の行と、外に出る DDR のボール（F10・M10・L11・M11）は使わない。DDR を休止させる信号は出していない（本番と同じく未対応） |
| 電圧 | I/O バンクの電圧（VCCIO）が 3.3 V でない場合 | FPGA は Low（と pp では High）を出すだけで、MCU は入力。3.3 V の MCU に 2.5 V の High が来ても壊れない。逆向きは起きない |
| 給電 | 基板の外での給電を間違えると、それ自体が壊す原因になる | **未確認・要調査。** GD-ROM 用の電源コネクタの電圧とピンを、Dreamcast に付けた状態でテスターで測ってから、同じ電圧を電流制限付きの電源で与える。SWD ヘッダの 1 番（3.3 V と推定）から入れる案は、レギュレータへの逆流の心配があり勧めない |

## 7. 本設計で決めたことの理由

- **番号は 1 バイト + 反転の 1 バイト**：雑音やクロックの線がたまたまフレームの形になっても、反転が合う確率は 1/256。クロック計測つきは和も付ける。
- **パルス幅の比で 0/1**：オシレータの周波数が 1 桁わからなくても読めるように。T は SYNC の 9T 全体から測るので、エッジの揺れの影響が小さい。
- **全レジスタの初期値 0**：AGM のフローが 0 以外の初期値をどう扱うかに依存しない。最初の状態（START）で 256 ティック待ってから始める。
- **ボール一覧は生成物**：Supra の pinmap をリポジトリに入れないため。番号の対応を固定するため一覧に SHA-256 を付け、各ツールが記録する。
- **SD に結果を書かない**：ファームに SD の書き込みが無い（`blockdev.rs` は読み出し専用）。RTT のログで足りる。

## 8. 確かめたこと（この PC で）

- `test_pinfind.py`（Amaranth 0.5.8 のシミュレーション、7 件）：走査順と波形のティック単位の一致、prescale 0、Low のボールを飛ばす、
  `max_loops` で止まって以後出力しない、大きな番号、クロックプローブのカウント、`gen.py` の出力（全ポートにボールが 1 つずつ・コンフィグのボールを使わない、3 つのクロック源）。
  毎クロック「出力が 2 本以上にならない」「FRAME 以外で出力しない」を確かめている。
- `common/pinfind`（`cargo test`、17 件）：3〜100,000 単位/ティックの全範囲、ドリフト、±1/4 ティックの揺れ、反転、クロック・乱数が誤ってフレームにならない、
  グリッチ、時刻の折り返し、途中で切れたフレーム、クロック部の往復、**Amaranth のシミュレーションの波形（`testdata/sim_edges.txt`）を復号して番号・T・周波数が合う**、ロジアナ CSV の読み込み。
- `tools/pinfind/tests`（`python -m unittest`、12 件）：作り物の pinmap で一覧、除外、`pins2asf.py` の完全・不完全・衝突・`build.sh` と同じ検査。
- ファーム：`--no-default-features --features pinfind` がビルドできる（ビットストリームの代わりに中身が無意味な 552,352 B のファイルを一時的に置いて、
  探索のコードがリンクされることも確認。text 35,384 B、`PINS` 8,448 B、スタックの余裕 約 6.6 KB）。サンプリングのループは逆アセンブルで 30〜40 命令（約 2 MHz と推定。測っていない）。
- 確かめていないこと：Verilog の上位（`PinFind.v`）を Yosys や Supra に通すこと（この PC の Yosys は Amaranth 内蔵の縮小版で Verilog を読めない）、ビットストリーム、実機のすべて。
