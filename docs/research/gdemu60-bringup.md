# GDEMU 6.0 を ERASE したあとの立ち上げ手順（純正に戻れない前提）

2026-11-22、Opus（文書・ファームの `census` 機能・ホストテストのみ。実機の操作はしていない）。
前提：LOG の判断の記録（2026-11-22）「6.0 の MCU はロック（GPNVM0）とみなす。純正のバックアップはできない。ERASE は今は行わない（ユーザーが決める。2 枚目の基板もありうる）」、R177（SWD の DP が無応答）。
根拠の印：**[一次]** データシート・本リポジトリのコード・probe-rs 0.32.0 のソース／**[二次]**／**[推測]**／**[不明]**。

**この文書は ERASE を勧めるものではない。** ERASE するかどうか、どの基板で行うかはユーザーが決める。
目的は、もし ERASE したとき（この基板でも 2 枚目でも）に、**純正に戻る手段が一切ないまま**でも、立ち上げをできるだけ速く・安全に進められるようにしておくこと。

関連：`INSTALL.md` §3・§4・§6・§7、`docs/design/fw-bootloader.md` §1・§3-4、`docs/research/ag16k-config.md` §1・§7・§8、`docs/design/g1-ag16k-port.md` §1-5・§2-4・§4-2、
`fpga/pinfind/README_ja.md`・`FIRST_RUN_ja.md`、`tools/fwbackup/README_ja.md`、`docs/research/gdemu60-install-routes.md`（ブランチ `docs/install-routes`）、LOG R115・R116・R142〜R145・R162・R169・R177。

---

## 0. 要約

1. **ERASE が消すのは MCU の中だけ**：フラッシュ全体（純正の MCU ファームと、おそらく純正の FPGA ビットストリームも）、全ロックビット、GPNVM0（セキュリティ）と GPNVM1（起動元）。
   消えた直後は **ROM の SAM-BA から起動**する。ユニーク ID・キャリブレーション・FPGA・SD カードは変わらない（§1-1）。
2. **ERASE の直後にやること**は、SWD で DP → CHIPID `0x281A0761` → GPNVM と ロックビットが 0 であることを読むだけ。
   最初に書くのは NetGD ではなく、**何も駆動しないファーム `census`**（このブランチで追加）。`probe-rs download` は書き込みの最後に **GPNVM1 を自分で立てる**ので、手で GPNVM を書く必要は無い（§1-3）。
3. **順番**：今（ロックのまま）できる測定 → ERASE → SWD の確認 → `census` → ピン探し 段階 0 → 1 → 2（→ 3）→ 実ピンの `.asf` → `fpga/agm/build.sh --pins`（RETIMING オフ）→ NetGD `board-v60` を基板の外で → Dreamcast（§2）。
4. **今、無電源でできる測定**（§2-1）で、PS の 5 本の候補・SWD ヘッダの 1 番（3.3 V）と 6 番（NRST）・LED・ボタン・SD・UART・プルアップ抵抗のかなりの部分が ERASE の前に決まる。
   ERASE の前にしかできないこと（**純正の PS の波形の記録**、純正ファームのままの基板外給電の予行）もある（§2-2）。
5. **止まる条件**は段階ごとに §3 にまとめた。とくに「ERASE のあとも DP が無応答」なら原因はロックではなかったことになり、その基板は純正も NetGD も無い状態で止まる（§3-1）。
6. **用意するもの**：2 枚目の GDEMU 6.0（1 枚は純正のまま残す）、16 ch・100 MS/s 程度のロジアナ、針先プローブとルーペ、電流制限つきの安定化電源、予備の Pico（§4）。
7. 文書とコードの点検で、「バックアップが取れる」「ERASE の前に SWD が通る」を暗黙に前提にしていた箇所を直した（§5）。`INSTALL.md` の 6.0 の注記の大部分はブランチ `docs/locked-6.0` が直しているので、このブランチはそれと重ならない箇所だけを変えた。

---

## 1. ERASE が消すもの・消さないもの、直後の SWD

### 1-1 ERASE の効果

ERASE（LQFP-144 の 137 番。内蔵プルダウン約 15 kΩ、基準は VDDBU）を、**電源が入った状態で 220 ms を超えて High** にすると、フラッシュの再初期化が走る。100 ms 未満の High は無視される [一次：データシート 6430 §6.7・§8.2.3.4・§8.2.3.5・§8.2.4、`docs/research/web-prep-user-tasks.md` E-1、`docs/design/fw-bootloader.md` §1・§3-4]。

| 対象 | ERASE で | 根拠・補足 |
|------|----------|------------|
| フラッシュ全体（0x80000〜0x9FFFF、128 KB） | **すべて 0xFF**。純正の MCU ファームは永久に消える | [一次] |
| 純正の FPGA ビットストリーム | **消える見込み**。AG16KDDF256 は内蔵フラッシュを持たず、外付けのフラッシュも無いので、MCU のフラッシュの中にあり、MCU が PS で流していたはず | [推測：`ag16k-config.md` §0・§3] |
| GPNVM0（セキュリティビット） | **0 になる**（SWD でフラッシュ・SRAM・周辺に届く） | [一次] |
| GPNVM1（起動元） | **0 になる → 次の起動は ROM（SAM-BA）**。フラッシュに何か書いても、GPNVM1 を立てるまで動かない | [一次] |
| GPNVM2 | 2E は 1 プレーンなので意味を持たない。4E ではプレーンの入れ替え | [一次：`fw-bootloader.md` §1 の表] |
| ロックビット（全領域） | **すべて外れる** | [一次：`fw-bootloader.md` §1] |
| ユニーク ID・キャリブレーションビット | 変わらない | [一次：`web-prep-user-tasks.md` E-1] |
| GPBR（バックアップ領域のレジスタ） | ERASE の対象ではない。VDDBU が落ちれば消える（`pinfind` の GPBR3 など） | [推測] |
| FPGA（AG16K） | 何も変わらない。もともと電源を入れるたびに未構成から始まる | [推測：`ag16k-config.md` §3] |
| SD カード・他の部品 | 変わらない | — |

**ERASE で直らないもの**：JTAGSEL（142）・TST（138）の状態、配線、プローブ（Pico）の故障。R177 の DP 無応答の原因がこれらなら、ERASE のあとも SWD は通らない（§3-1、`gdemu60-install-routes.md` §4-6）。

### 1-2 ERASE の作業（参考。行うかどうかはユーザーが決める）

`INSTALL.md` §6 は v5.5 向けに「Dreamcast の電源で、H1 の 1 番（3.3 V）から ERASE パッドへ」と書いている。6.0 では次のようにするのがよい [推測を含む方針]。

- **基板の外で、電流制限つきの電源から給電して行う**（§2-2 の予行で電圧・電流の基準が取れてから）。Dreamcast の上ぶたを開けて電源部の近くで作業しなくて済む。
- 3.3 V は **SWD ヘッダの 1 番から 1 kΩ 程度の直列抵抗を通して**取る。ただし 1 番が 3.3 V であることはまだ推定（R116）なので、**§2-1 の N1 で先に確かめる**。
- ERASE パッドの位置は R177 で「U1 の 137 番の高さの横の銅のパッド」とみなしたが未確定。**パッド ↔ 137 番の導通は、ERASE をすると決めてから**、無電源で一瞬だけ確かめる（それまでは 137 番の周りに触れない）。
- 電源を入れた状態で 0.5〜2 秒 High に保つ → **線を外す** → 電源を切って入れ直す。線を付けたままだと、電源を入れるたびに消える（`INSTALL.md` §6 の 6）。
- 隣の 136（SHDN）・138（TST）に触れない。TST を電源投入時に High にすると FFPI のモードに入るおそれがある（`web-prep-user-tasks.md` E-2）。

### 1-3 ERASE の直後の SWD の手順

配線は今までと同じ **SWDIO（ヘッダ 4）・SWCLK（ヘッダ 5）・GND（ヘッダ 7）の 3 本**。§2-1 の N2 でヘッダ 6 = NRST が確かめられていれば、NRST を Pico の空きピンにつないでおくと、あとで `--connect-under-reset` が使える（確かめていなければ使わない。`INSTALL.md` §7）。
電源は基板の外の電流制限つきの電源（Dreamcast に付けない）。SD カードは抜く。probe-rs は **0.32.0**、チップ名は **必ず `ATSAM3U2E`**。

以下、`P="--chip ATSAM3U2E --protocol swd"` と書く（PowerShell なら `$P = '--chip','ATSAM3U2E','--protocol','swd'` にして `probe-rs read @P ...`）。

| # | コマンド | 期待 | 違ったら |
|---|----------|------|----------|
| 1 | `probe-rs list` | debugprobe が 1 台見える | プローブ側（`INSTALL.md` §2・§11） |
| 2 | `probe-rs info --protocol swd` | `Debug Port: DPv1 ...` が出る | **止まる**（§3-1）。原因はロックではなかった |
| 3 | `python tools/fwbackup/fwbackup.py check` | `UNLOCKED_2E`（CIDR `0x281A0761`） | `UNLOCKED_OTHER` なら止まる（別のチップ） |
| 3' | （手で）`probe-rs read $P b32 0x400E0740 1` | `400e0740: 281a0761` | 同上 |
| 4 | `probe-rs read $P b32 0x00080000 8` | 全部 `ffffffff`（消えている） | ERASE が効いていない。GPNVM を読む（5） |
| 5 | `probe-rs write $P b32 0x400E0804 0x5A00000D` → `probe-rs read $P b32 0x400E0808 2` | FSR（1 語目）の bit0 = 1（FRDY）、FRR（2 語目）= `00000000`（GPNVM0 = 0・GPNVM1 = 0） | GPNVM0 = 1 なら ERASE が効いていない（止まる） |
| 6 | `probe-rs write $P b32 0x400E0804 0x5A00000A` → `probe-rs read $P b32 0x400E080C 1` | `00000000`（ロックビットが全部外れている） | 0 でなくても `probe-rs download` は書く前に該当領域を外す（下）。記録だけ |
| 7 | `census` を書く：`probe-rs download $P --binary-format elf <census の ELF>` | 成功 | — |
| 8 | GPNVM をもう一度（5 と同じ） | FRR = `00000002`（GPNVM1 = 1、GPNVM0 = 0） | 下の「GPNVM1 が立たなかったら」 |
| 9 | `probe-rs reset $P` → `probe-rs attach $P <census の ELF>`（RTT。`| tee census.log`） | `PC GDEMU 6.0 pin census ...` が出る | §2-4 |

- 5・6 の `0x5A00000D`（GGPB）と `0x5A00000A`（GLB）は**読むだけのコマンド**で、何も書き換えない [一次：`mcu/bootcore/src/fcr.rs` のコマンド表、データシート §20.4.3]。
- **GPNVM1 は `probe-rs download` が立てる** [一次：probe-rs 0.32.0 `targets/SAM3U.yaml` の `atsam3u_128` を逆アセンブルして確認]：
  書き込みのあと（UnInit、機能 2）に GGPB で GPNVM を読み、GPNVM1 が 0 なら `0x5A00000D + 0xFE = 0x5A00010B`（SGPB の引数 1）を出す。続けて CGPB 2（`0x5A00020C`）。
  書き込み（ProgramPage）は各ページの前に CLB（ロックの解除）→ EWP を出すので、ロックビットが残っていても書ける。上流の `INSTALL.md` §6 → §7 が GPNVM に触れずに動いていたのはこのため。
- **GPNVM1 が立たなかったら**（8 で FRR = 0）：手で `probe-rs write $P b32 0x400E0804 0x5A00010B` を **この文書からコピーして**実行し、8 をやり直す。
  手で打たない理由：**`0x5A00000B`（SGPB の引数 0）はセキュリティビットを立てる**語で、1 文字違い。立てると SWD がまた閉じ、ERASE をもう一度するしかない。
- **OpenOCD は使わない**：`flash write_image` は GPNVM1 を立てない（`at91sam3 gpnvm set 1` が別に要る）。`gpnvm set 0` と打ち間違えるとロックされる。この手順は probe-rs だけで書く。

### 1-4 してはいけないこと（ERASE の後）

- `0x5A00000B` を書く・OpenOCD の `at91sam3 gpnvm set 0`・GPNVM を書き換える ELF（0x1FFFFFF0〜0x1FFFFFFF にデータがあるもの。probe-rs の `atsam3u_gpnvm` アルゴリズムが使われる）を書く。
- `--chip ATSAM3U4E` を使う・`cargo run` で書く（`mcu/netgd/.cargo/config.toml` の runner は `ATSAM3U4E` のまま。6.0 では `probe-rs download --chip ATSAM3U2E` を直接使う）。
- **最初に NetGD（`board-v60`）を書く**：NetGD は起動直後に SMC・PCK0（PA21）・UART（PA12）・HSMCI・PS の 3 本を駆動し、失敗時は LED（PB18）も出力にする。どれも 6.0 では v5.5 からの仮定（§2-5）。先に `census` とピン探しで確かめる。
- `resident` のイメージを書く（6.0 では常駐部をロックしない判断 E6 のとおり、最初はアプリ単体を 0x80000 に置く）。
- ERASE の線を付けたまま電源を入れる。NRST を確かめずに `--connect-under-reset` を使う。
- ピン探しで FPGA が構成されている間にデバッガでコアを止める（`fpga/pinfind/README_ja.md` §6）。
- §3 の条件が揃う前に Dreamcast に付ける。

---

## 2. 立ち上げの計画（順番）

| 段階 | いつ | 何をする | 決まること | 基板の状態 |
|------|------|----------|------------|------------|
| **N** | 今（ロックのまま） | 無電源のテスターでの測定・写真（§2-1） | ヘッダ 1・6、PS の候補、LED・ボタン・SD・UART、プルアップ | 純正のまま |
| **N+** | 今（ロックのまま、任意） | 電源ありで読むだけ：純正の PS の波形、基板外給電の予行（§2-2） | PS の作法（時間・ビット数・ビット順・nCONFIG の幅）、給電の基準値 | 純正のまま |
| **E** | ユーザーが決めたら | ERASE（§1-2） | — | 空 |
| **S** | E の直後 | SWD の確認（§1-3 の 1〜6） | DP・CHIPID・GPNVM・ロック | 空 |
| **C** | S の後 | `census`（§2-4） | PS の 5 本・ボタン・カード検出の当否、各ピンの状態 | census |
| **P0〜P3** | C が合格したら | ピン探し 段階 0 → 1 → 2（→ 3）（`FIRST_RUN_ja.md`） | FPGA のボール ↔ MCU のピン（SMC・IRQ・RSTn・クロック） | pinfind |
| **L** | P2 の後、別の回 | CN1 側をロジアナで（`fpga/pinfind/README_ja.md` §5） | FPGA のボール ↔ G1 コネクタ（DC_* 34 本） | pinfind |
| **B** | P2・L の後 | 実ピンの `.asf` → `fpga/agm/build.sh --pins`（§2-6） | 本番のビットストリーム | — |
| **M** | B の後 | NetGD `board-v60` を基板の外で（§2-7） | `FPGA debug: primary=0xc0de`、SD | NetGD |
| **D** | M が合格したら | Dreamcast に付ける | BIOS が起動する | NetGD |

### 2-1 今できる測定（ロックのまま・無電源）

**共通の条件**：基板を Dreamcast から外し、SD カードを抜き、数分おいてコンデンサを放電させる。テスターは抵抗レンジか導通モード。黒（COM）は GND（ヘッダ 7 番か SD スロットの金属の外枠）に固定し、赤で探る。
電源の入っていないチップでは ERASE は動かない（チップに電源が無く、消去を実行できない [一次からの推論、`web-prep-user-tasks.md` E-2]）。それでも **137 番（ERASE）とその隣（133〜142 番）は原則として触らない**。触る必要がある項目（下の ★）は最後に回し、針先プローブとルーペで、1 回ずつ短く当てる。
抵抗値はチップの保護ダイオードを通るので目安にしかならない。値と、赤・黒を入れ替えた値の両方を記録する。

ピン番号は LQFP-144 のもの（`docs/research/user-tasks.html`・`web-prep-user-tasks.md` A の表、データシート表 4-1）。U1 の 1 番ピンは「U1」の白丸の角（R177）。
**PA1・PA3〜PA8・PA22・PA30 の番号は本リポジトリにまだ書かれていない**ので、データシート表 4-1 で引いてから当てる（下では「表 4-1」と書く）。

| # | 当てる場所 | 期待・分かること | リスク |
|---|------------|------------------|--------|
| N1 | SWD ヘッダ **1 番** ↔ SD スロットの **4 番の接点（VDD）**、1 番 ↔ GND の抵抗 | 導通すれば 1 番 = 3.3 V の網で確定（ERASE の 3.3 V 源・ロジアナのプルアップ先）。GND との間が 0 Ω でないこと | 無し |
| N2 | ヘッダ **6 番** ↔ U1 の **11 番（NRST）** | 導通すれば `--connect-under-reset` が使える（ERASE の後、壊れたファームから戻る保険） | 低（11 番は SWD の 7・9 番と同じ辺。R116 と同じ作業） |
| N3 | ヘッダ 2 番 ↔ R4 の反対側、3 番 ↔ R3 の反対側（裏面）。余力があれば R4 ↔ U1 の 1 番（TDI）、R3 ↔ 4 番（TDO） | JTAG で IDCODE を読む案（`gdemu60-install-routes.md` §4-4）の準備 | 中（1 番は 144 番の角の隣。137 番には近づけない） |
| N4 | PS の 3 本の**プルアップ**：U1 の **55 番（PB1）**・**57 番（PB2）**・**106 番（PA25）** ↔ ヘッダ 1 番（3.3 V）の抵抗 | AGM の手引きどおりなら **PB1 ≈ 1 kΩ（CONF_DONE）、PB2 ≈ 10 kΩ（nSTATUS）、PA25 ≈ 10 kΩ（nCONFIG）**（`ag16k-config.md` §1-2・§7 Q4〜Q6）。3 本がこの形なら v5.5 の割り当ての強い傍証 | 低（55・57 は 2 番目の辺、106 は 3 番目の辺。137 から遠い） |
| N5 | **103 番（PA23）・105 番（PA24）** ↔ ヘッダ 1 番・GND の抵抗 | DATA0・DCLK には普通プルは無い。どちらかに強いプルがあれば記録 | 低 |
| N6 | 裏面の U2 の真下のビアと、MCU の 103・105・106・55・57 番の導通（`g1-ag16k-port.md` §2-4 の方法 1）。ボール表を左右反転して重ねる | **G2（DATA0）・G1（DCLK）・H3（nCONFIG）・F16（CONF_DONE）・F3（nSTATUS）** に当たれば PS の 5 本が確定 | 低（ビアがレジストで覆われていれば不可。削らない） |
| N7 | 裏面のビアで **F15（MSEL1）↔ GND** | GND なら PS で確定（`ag16k-config.md` Q1、最重要）。3.3 V なら AS（どこかにフラッシュがある） | 低 |
| N8 | **D1（LED）** の両方のパッド ↔ GND・↔ 3.3 V・↔ U1 の **25 番（PB18）**。D1 の近くの直列抵抗の両端も | どちらの足が MCU 側か、**極性**（MCU が Low で点灯か High で点灯か）。25 番に行かなければ、写真で配線を追って MCU の何番に行くかを記録 | 低 |
| N9 | **ボタン**の 4 本の足：どの組が GND か、残りの組 ↔ U1 の **101 番（PA20）**、プルアップ抵抗の有無（↔ 3.3 V の抵抗） | ボタンのピンと、外付けのプルアップがあるか（無ければファームが内蔵プルアップを使う。今のコードは使う） | 低 |
| N10 | **SD スロット**の 9 本の接点（1 = DAT3、2 = CMD、3 = GND、4 = VDD、5 = CLK、6 = GND、7 = DAT0、8 = DAT1、9 = DAT2）と、カード検出・書き込み禁止のスイッチの足 ↔ U1。期待は CLK = PA3、CMD = PA4、DAT0〜3 = PA5〜PA8、カード検出 = PA1（番号は表 4-1） | HSMCI は SAM3U ではこのピンにしか出ないので、SD が動いているなら 6 本は確定のはず。決めたいのは**カード検出のピン**と、各線のプルアップの値 | 低（PA3〜PA8 がどの辺かは表 4-1 で確かめ、137 の近くなら ★ 扱い） |
| N11 | **3 ピンヘッダ**（U1 の上）：各ピン ↔ GND・3.3 V。★ 次に ↔ U1 の **133 番（PA11）・134 番（PA12）** | v5.5 の H4 と同じ UART（1 = RX、2 = TX、3 = GND）か。UART があればログの出口になる | **★ 高め（133・134 は 137 の 3〜4 本隣）**。ヘッダ側で GND・3.3 V を先に決め、MCU 側は最後に 1 回ずつ |
| N12 | **5 ピンヘッダ**（上辺）：各ピン ↔ GND・3.3 V の抵抗、↔ U1 の 69〜72 番（PC28〜PC31、v5.5 の H3） | v5.5 の H3 か、FPGA の JTAG（TCK は 1 kΩ で GND、TMS・TDI は 10 kΩ で 3.3 V の形が多い。`ag16k-config.md` Q8）か | 低 |
| N13 | **2 ピン**（ロゴの横、何かがはんだ付けされている）：各ピン ↔ GND・3.3 V・CN1 の電源ピン | 電源か、ボタン・LED の別の口か | 低 |
| N14 | 電源の網：CN1 の電源ピン・U4（1117-1.2）・Q1（6201）・220 µF・ヘッダ 1 番の導通、各レール ↔ GND の抵抗（`FIRST_RUN_ja.md` D-1 と同じ） | 基板外給電に入れる場所と電圧の見当（§2-2 の予行の準備） | 低（12 V の網があっても無電源なら害は無い） |
| N15 | 裏面の R5〜R9・RA・R3（Y1 の近く）の両端 ↔ U1・GND・3.3 V | どれが PS・ボタン・SD・ERASE/JTAGSEL のプルかの見当 | 低 |
| N16 | 写真：両面を斜めの光で高解像度に。とくに U1 の 1 番の角の周り（ERASE パッドの候補）、D1、ボタン、SD スロットの配線、U2 の下のビア（レジストで覆われているか） | ERASE パッドの位置を触らずに絞る。方法 1 のビアの当たりを付ける | 無し |
| N17 ★ | JTAGSEL（**142 番**）・TST（**138 番**）↔ GND の抵抗（`gdemu60-install-routes.md` §4-2） | どちらかが High 側なら、ERASE をしても SWD は通らない見込み → **ERASE の前の判断材料として価値が高い** | **★ 最も高い（138 は 137 の隣、142 は 5 本隣）**。できれば裏のビアで。表で当てるなら最後に、ルーペで、1 回ずつ |
| — | ERASE パッド ↔ 137 番 | **今はしない**（ERASE を決めてから。§1-2） | — |

結果は表にして LOG（ユーザーが記録）と `docs/boards/gdemu60/README.md` のチェックリストに残す。**N4・N6・N7 のどれかが v5.5 の割り当てと食い違えば、`cyclone::pinmap` と `embassy-atsam3` の `UserPeripherals` を直してからでないと、ピン探しのファームも NetGD も書かない**（型で固定してあるので、直し忘れはコンパイルエラーになる）。

### 2-2 今できること（ロックのまま・電源あり・読むだけ。任意だが ERASE の前にしかできない）

純正ファームが動いている間だけ取れる情報がある。ERASE の後は二度と取れない。どれも純正ファームを変えない。

| # | 何を | 方法 | 分かること | リスク |
|---|------|------|------------|--------|
| N+1 | **純正の PS の波形**（`ag16k-config.md` Q13〜Q15、`g1-ag16k-port.md` §1-5） | 無電源のうちに細いエナメル線を 105 番（DCLK）・103 番（DATA0）・106 番（nCONFIG）・55 番（CONF_DONE）・57 番（nSTATUS）の根元か、N6 で見つけたビアにはんだ付け → ロジアナ（3.3 V、DCLK を取りこぼさない速さ）→ 電源投入から CONF_DONE が上がるまで記録 | DCLK のクロック数（4,418,928 なら非圧縮）、周波数、流している時間（Dreamcast が待てることが実証済みの長さ）、ビット順、nCONFIG の幅、CONF_DONE の後のクロック数。**`psload::AG16K` の値と `ps-msb-first` の要否がほぼ決まる** | 中（はんだ付けは無電源で。記録したビット列は純正のビットストリームそのものなので、**数（ビット数・周波数・時間）だけを残し、リポジトリにも Claude にも渡さない**） |
| N+2 | 純正の LED の点き方と、どのピンが動くか | N+1 の線に 25 番（PB18）を足すか、N8 で決めた足に | LED のピンと極性の確認 | 低 |
| N+3 | 3 ピンヘッダの出力 | ロジアナで、電源投入から（読むだけ。駆動しない） | 純正が UART に何か出しているか、ボーレート | 低 |
| N+4 | **基板外給電の予行**（`FIRST_RUN_ja.md` D、R143） | N14 の結果から、CN1 の電源ピンに電流制限つきの電源で同じ電圧を入れる（100〜150 mA の制限から） | **純正ファームのまま**基板の外で起動するか、電流と電圧の基準値（FPGA が構成された状態の電流を含む） | 中（12 V に触れない、制限の無い電源を使わない） |
| N+5 | gdprobe の純正の記録で足りないもの | `tools/gdprobe`（R117〜R179 の続き） | NetGD との差分テストの正解データ | 無し |

N+1〜N+4 は、2 枚目の基板を用意できるなら、**純正のまま残す 1 枚**で後からでも取れる。1 枚しか無いなら ERASE の前に済ませる。

### 2-3 ERASE（ユーザーの判断）

§1-2・§1-3。**ERASE の前に揃えておくもの**（§3-0）が全部揃ってから。

### 2-4 `census`：ERASE の後で最初に書くファーム（このブランチで追加）

- 何をするか：全 PIO を**入力のまま**にし（リセット直後と同じ）、内蔵プルアップを入れた状態と切った状態で全 96 本を読む。各ピンを分類して RTT に出し、v5.5 の役割と照らす。
  そのあと 30 秒、ピンの変化を見張る（**この間にボタンを押す、SD カードを抜き差しする**）。これを繰り返すので、いつ RTT をつないでもよい。
  **どのピンも駆動しない**（書くレジスタは ODR・IDR・MDDR・IFDR・PER・PUER・PUDR だけ）。FPGA は構成しない（ビットストリームを埋め込まない）。LED も使わない。
- 作り方・書き方：
  ```
  cd mcu/netgd
  cargo build --release --no-default-features --features census
  probe-rs download --chip ATSAM3U2E --protocol swd --binary-format elf target/thumbv7m-none-eabi/release/netgd
  probe-rs reset    --chip ATSAM3U2E --protocol swd
  probe-rs attach   --chip ATSAM3U2E --protocol swd target/thumbv7m-none-eabi/release/netgd | tee census.log
  ```
  （`DEFMT_LOG=off` にはしない。ログが本体。`--target-dir` は使わない。この PC でのビルドは .text 8.8 KB。）
- 分類（`common/pincensus`、ホストテスト 12 件）：

  | 分類 | プルアップあり | プルアップなし | 意味 |
  |------|----------------|----------------|------|
  | `low` | 常に 0 | 常に 0 | GND・プルダウン・相手が Low を出している（**未構成の FPGA の CONF_DONE**） |
  | `high` | 常に 1 | 常に 1 | 外付けのプルアップ・相手が High・3.3 V。**浮いていて電荷が残っている線とは区別できない**（SAM3U にプルダウンが無いため） |
  | `float` | 常に 1 | 0 か両方 | 何もつながっていないか、相手がハイインピーダンス |
  | `toggle` | 変化が多い | | クロックや動いている線 |
  | `unstable` | 両方、変化が少ない | | 読んでいる間に変わった（ボタンなど） |
  | `odd` | 常に 0 | 常に 1 | 2 つの読みの間に変わった。次の回を見る |

- **合否（C の go/no-go）**：`PC ps-check agree=6 disagree=0`。中身は v5.5 の仮定が未構成の FPGA で見せるはずの形：
  **PB1（CONF_DONE）= `low`、PB2（nSTATUS）= `high`、PA25（nCONFIG）= `high` か `float`（`low` でない）、PA23・PA24 = `high` か `float`、PA20（ボタン、押していない）= `high` か `float`**。
  CONF_DONE が未構成の間 Low なのは Cyclone の作法で、AGM の資料には書かれていない [推測]。`low` にならなくても、それだけで v5.5 の割り当てが誤りとは言えないが、**`disagree` が 1 以上ならピン探しに進まない**で、§2-1 の N4・N6 をやり直す。
- ほかに見るもの：見張りの間に `PC change ... pin=PA20` が出ればボタンは PA20、SD の抜き差しで変わるピンがカード検出（v5.5 は PA1）。
  `toggle` のピンは、外からクロックが来ているピン（CPU の 12 MHz の水晶は専用ピンなので出ない。11.2896 MHz がもし MCU にも来ていればここに出る）。
  `low` が多数ある、PS 以外の SMC の線が `low`・`toggle` になっている、などは記録して LOG で判断する。
- ファーム無しで同じことを SWD だけでする方法（参考）：ERASE の直後（SAM-BA が ROM で動いている）でも、
  `probe-rs read $P b32 0x400E0418 1`（PMC_PCSR0。bit10〜12 = PIOA〜C のクロック）、必要なら `probe-rs write $P b32 0x400E0410 0x00001C00`（PIO のクロックを入れるだけ）、
  `probe-rs read $P b32 0x400E0C3C 1`・`0x400E0E3C`・`0x400E103C`（PIOA〜C の PDSR）で入力の値が読める。ただし SAM-BA は UART（PA11・PA12）などを自分で設定する [推測：データシート §20 の SAM-BA の章] ので、`census` を書いてから読む方がきれい。

### 2-5 `board-v60` の MCU 側でまだ仮のピンと、その決め方

`mcu/netgd/src/cyclone.rs` の `pinmap`、`mcu/embassy-atsam3/src/lib.rs` の `UserPeripherals`・`smc.rs`・`hsmci.rs`・`uart.rs`、`ide.rs`（PA22）が v5.5 の値を使っている。6.0 で確定しているのは SWD だけ（R116）。

| 線 | 今の値（v5.5） | NetGD が駆動するか | 今（ロックのまま）決める方法 | ERASE の後で決める方法 |
|----|----------------|--------------------|------------------------------|------------------------|
| PS DATA0 | PA23（103） | 出力（構成中） | N5・N6、N+1 の波形 | census（`high`/`float`）→ 段階 0 で CONF_DONE が上がる |
| PS DCLK | PA24（105） | 出力（構成中） | N5・N6、N+1 | 同上 |
| PS nCONFIG | PA25（106） | **出力（常に）**。ピン探しの安全装置もこれに頼る | **N4（≈10 kΩ）**・N6、N+1 | census（`low` でない）→ 段階 0 |
| PS CONF_DONE | PB1（55） | 入力 | **N4（≈1 kΩ）**・N6 | **census（`low`）** → 段階 0 の `PF FPGA configured` |
| PS nSTATUS | PB2（57） | 入力 | **N4（≈10 kΩ）**・N6 | census（`high`） |
| MSEL1 | （FPGA の F15） | — | **N7** | 段階 0 で構成できれば PS で確定 |
| LED | PB18（25） | 出力（失敗時の点滅） | **N8**、N+2 | ピン探しの段階 2 で PB18 が FPGA のボールに**つながっていない**ことを確かめてから、点滅を試す |
| ボタン | PA20（101） | 入力（プルアップ） | **N9** | **census の見張り** |
| SD（HSMCI） | PA3〜PA8（固定） | HSMCI | **N10** | NetGD の `Card up` |
| カード検出 | PA1 | 入力 | **N10** | **census の見張り（抜き差し）** |
| UART | PA11・PA12（133・134） | **PA12 を出力**（`uart::init` が起動時にバナーを送る） | ★ N11、N+3 | census（PA11 が `high` なら相手が駆動している）、段階 2 で FPGA に行っていないこと |
| FPGA 48 MHz | PA21（102、PCK0）。`docs/PCB.md` は PA28（45） | **PA21 を出力**（PCK0） | N6 のビアで 102・45 番 ↔ 専用クロックのボール（`ag16k-config.md` Q9） | 段階 2・3（クロックボールの周波数、MCU 側の対応） |
| FPGA → MCU 割り込み | PA22（表 4-1） | 入力 | N6 のビア | 段階 2 |
| FPGA のリセット | PA30（表 4-1） | 入力（RSTn のエッジを数える） | N6 のビア | 段階 2 |
| SMC（D0〜D15、A0〜A8、NRD、NWE、NCS3、NBS1） | PB6〜PB16・PB19・PB23・PB25〜PB31・PC0〜PC6・PC15・PC17 | **出力・双方向** | N6（全部は時間がかかる） | **段階 2**（MCU 側の対応が全部出る）→ M の `0xC0DE` で最終確認 |
| SWD | ヘッダ 4・5・7（確定） | — | 済（R116・R177） | — |
| NRST | ヘッダ 6（推定） | — | **N2** | — |

**MCU のピン機能は固定**なので（SMC・HSMCI・UART・PCK0 は決まったピンにしか出ない）、6.0 で変わりうるのは「どの FPGA のボールにつながるか」と、PS・LED・ボタン・IRQ・RSTn・カード検出のような GPIO の線だけ（`g1-ag16k-port.md` §4-2）。
GPIO の線が違っていたら `cyclone::pinmap` と `UserPeripherals` の型を直す（型で固定してあるので片方だけ直すとコンパイルエラー）。

### 2-6 実ピンのビットストリーム

1. 段階 2（と L）の `rtt.log`・ロジアナの記録から `tools/pinfind/pins2asf.py` で本番の `.asf` を作る。1 本でも欠けると `NETGD-PROVISIONAL-PINS` の印が付き、終了コード 2（R142）。
2. WSL で `SUPRA_HOME=... fpga/agm/build.sh --pins <その .asf>`。**RETIMING は既定でオフ**（R169。`--supra-retiming` は試験専用で、付けると CDDA が無音になる）。
   報告の `retiming off` の行と、map.log に DFFLEGALIZE の前の ABC が無いこと（`build.sh` が止まる）を確かめる。
3. 出力 `fpga/agm/output/OpenGDEMU_ag16k.rbf`（`-provisional` の付かない名前だけ）を `build.rs` が拾う（`RBF_V60`）。
4. `checkpnr.py` と同じ考えで、配置後の「ポート → ボール」が `.asf` と一致することを確かめる（ピン探しの `build.sh` は自動。本体の `build.sh` で同じ確認が要るかは B の段階で判断）。

### 2-7 NetGD `board-v60` の最初の起動（基板の外 → Dreamcast）

- 最初は**常駐部なし**：`cargo build --release --no-default-features --features board-v60`（ログあり）→ `probe-rs download --chip ATSAM3U2E ...` → RTT。
- 基板の外（電流制限つきの電源、SD あり）で：`FPGA Loaded!`（CONF_DONE が上がる。`early done` の値も記録）→ **`FPGA debug: primary=0xc0de secondary=0xc0de`**（SMC のアドレス・データ・ストローブが全部正しい印。`g1-ag16k-port.md` §2-4 の方法 4）→ `Card up` → イメージの一覧。
- それが揃ってから Dreamcast に付け、BIOS → GDmenu → gdprobe（純正の記録との比較）。
- 常駐部（`resident`）とロックは、6.0 のピンが全部確定してから（判断の記録 E6）。

---

## 3. 進む・止まるの判断

### 3-0 ERASE の前に揃えるもの（どれか欠けたら ERASE しない）

- [ ] §2-1 の N1（ヘッダ 1 = 3.3 V）・N2（ヘッダ 6 = NRST）・N4（PS のプルアップ）・N7（MSEL1）が済んでいる。N17（JTAGSEL・TST）は可能なら。
- [ ] Pico の自己テスト（`gdemu60-install-routes.md` §4-1）で、プローブ側が正しく SWD を出せることを確かめてある。**ここが未確認のまま ERASE して DP が無応答だと、原因が切り分けられない。**
- [ ] 1 枚しか無いなら §2-2 の N+1・N+4 が済んでいる（2 枚あるなら純正の 1 枚で後からでよい）。
- [ ] `census` の ELF と、ピン探しの段階 0 の ELF（`tools/pinfind/mkstage.ps1 -Stage 0`）がビルド済みで、SHA-256 を控えてある。
- [ ] 電流制限つきの電源・ロジアナ・プルアップ用の 10 kΩ・ERASE 用の 1 kΩ が手元にある（§4）。
- [ ] ユーザーが、純正の MCU ファーム・FPGA のビットストリーム・純正の更新経路が永久に失われ、NetGD が動くまでその基板が使えないことを受け入れている。

### 3-1 ERASE の直後（S）

| 結果 | 判断 |
|------|------|
| DP が見え、CHIPID = `0x281A0761`、GPNVM = 0、フラッシュが `ff` | **進む**（census を書く） |
| **DP が無応答のまま** | **止まる。** 原因はロックではなかった（JTAGSEL・TST・配線・プローブ）。ERASE をもう一度しても意味は無い。N17・Pico の自己テスト・ロジアナで SWD の ACK を見る（`gdemu60-install-routes.md` §4-5）。この基板は純正も NetGD も無い状態 |
| CHIPID が違う | 止まる（チップが想定と違う。`fwbackup` は `UNLOCKED_OTHER`） |
| GPNVM0 = 1、またはフラッシュが `ff` でない | ERASE が効いていない。線を外して電源を入れ直し、1 回だけやり直す。それでも同じなら止まる |
| download の後で GPNVM1 = 0 | §1-3 の手順で手で立てる（コピーして実行）。それでも 0 なら止まる |

### 3-2 census（C）

| 結果 | 判断 |
|------|------|
| `ps-check agree=6 disagree=0`、見張りでボタン（とカード検出）が分かった | **進む**（ピン探しの段階 0。`FIRST_RUN_ja.md` F） |
| `disagree` が 1 以上 | **止まる。** その線について N4・N6 をやり直し、必要なら `pinmap` を直してからもう一度 census |
| 電流が予行（N+4）の MCU だけの値から大きく増えた | すぐ電源を切る（census は何も駆動しないので、増える理由が無い） |
| RTT が出ない | GPNVM1・`reset` を確かめる。`--connect-under-reset`（N2 が済んでいれば） |

### 3-3 ピン探し（P0〜P3）

`FIRST_RUN_ja.md` §F の停止条件と、`fpga/pinfind/README_ja.md` §0-2 の「次に進む条件」のとおり。加えて：
- 段階 0 で `PF FPGA configured` が出ない → PS のピンかビット順（`ps-msb-first`）か PS の作法。N+1 の波形があれば先にそれと比べる。
- 段階 2 で LED（PB18）・UART（PA12）・PCK0（PA21）が FPGA のボールにつながっていると出たら、NetGD はそのピンを今のまま駆動してはいけない（§2-5）。

### 3-4 実ピンのビットストリームと NetGD（B・M・D）

| 結果 | 判断 |
|------|------|
| `pins2asf.py` が終了コード 2（欠けたピン） | 止まる。足りない線を段階 2・L でもう一度 |
| `build.sh` が `retiming off` を出さない・map.log の確認で止まる | 止まる（R169） |
| `FPGA debug` が `0xc0de` でない | SMC のどこかが違う。Dreamcast に付けない |
| 基板の外で全部そろった | Dreamcast に付けてよい |

### 3-5 どの段階でも

- 電流制限に当たった、基準より大きく増えた、熱・におい・音 → すぐ電源を切る。
- ログが途中で止まった、ELF やビットストリームの SHA-256 が控えと違う → 進まない。
- 迷ったら進まない。結果はユーザーが LOG に記録する。

---

## 4. 用意するもの

| 物 | 目安 | 何に使う |
|----|------|----------|
| **2 枚目の GDEMU 6.0** | 同じ版（U1 = ATSAM3U2E、U2 = AG16KDDF256、「GDEMU 6.0」の印字）。できれば同じ売り手 | 1 枚は**純正のまま残す**（N+1 の波形・gdprobe の正解データ・いつでも比較できる基準）。もう 1 枚を ERASE する。いちばん後悔の少ないやり方 |
| ロジックアナライザ | **16 ch・100 MS/s 以上・3.3 V の閾値・入力は高インピーダンス**（sigrok / PulseView で読めるもの）。8 ch・24 MS/s の安価なものでも PS の 5 本と SWD は読める | N+1（DCLK の速さは不明なので速い方が安心）、ピン探しの段階 0〜2 の監視、CN1 側の 34 本（`fpga/pinfind/README_ja.md` §5。16 ch なら 3 回に分ける） |
| 細いプローブ | 0.5 mm ピッチ用の針先プローブ（テスター棒に付けるもの）、ばね式の極細テストクリップ（マイクログラバー）を 6〜8 本 | N の測定、ロジアナを LQFP のピンやビアにつなぐ |
| 拡大 | ルーペ（10 倍）か USB 顕微鏡 | 137 番の周りを避けて当てる、写真（N16） |
| はんだ付け | 細いこて先、0.1〜0.2 mm のエナメル線（UEW）、フラックス、カプトンテープ | N+1 の線を無電源で付けておく（電源ありでクリップを当てない） |
| 安定化電源 | 可変（0〜6 V 程度）、**電流制限を 10 mA 単位で設定できる**もの、電流の記録（電源の記録機能か USB の電流計） | 基板外給電（N+4・ピン探し・census・NetGD の基板外の試験）、ERASE。12 V は使わない |
| 2 台目の Pico（RP2040） | debugprobe を入れたものと同じでよい | Pico の自己テスト（`gdemu60-install-routes.md` §4-1）、予備のプローブ |
| 抵抗 | 10 kΩ を 10 本程度、1 kΩ を数本 | ロジアナで読む線の外付けプルアップ（`FIRST_RUN_ja.md` F-4）、ERASE の直列抵抗 |
| （任意）JTAG のプローブ | FT232H / FT2232H 系 | ヘッダ 2・3 番の JTAG で IDCODE（`gdemu60-install-routes.md` §4-4）。ERASE の前の判断材料 |
| その他 | ESD のマット・リストストラップ、予備の SD カード、ジャンパー線 | — |

---

## 5. 点検：バックアップや ERASE 前の SWD を暗黙に前提にしていた箇所

| # | 場所 | 前提にしていたこと | 対応（このブランチ） |
|---|------|--------------------|----------------------|
| A1 | `fpga/pinfind/FIRST_RUN_ja.md` 冒頭・§B・§E | 手順 B（`fwbackup` で dump）が通ることが先へ進む条件。ロックされた基板の分岐が無い。E-3 の書き込みは「未確定」 | ロックされた基板の分岐 B'（バックアップ無し、ERASE はユーザーの判断、ERASE の後は本書 §1-3 → census → 段階 0）を追加。E-3 に具体的な手順（本書 §1-3）を書いた |
| A2 | `fpga/pinfind/README_ja.md` §0-1 の 1 | 同上 | 同上の分岐を 1 行で追加 |
| A3 | `tools/fwbackup/README_ja.md` 「probe-rs の download / erase / reset などを手で実行してはいけません」 | 純正が残っている基板だけを想定。ERASE の後の手順と区別が無い | 「純正が残っている間は」に限り、ERASE の後は本書に従うと追記。ERASE の後の `check` は CHIPID の確認に使えることも追記 |
| A4 | `INSTALL.md` §6「Confirm it worked」→ §7 | GPNVM1 に触れていない（probe-rs の `download` が立てるので動くが、OpenOCD の手順では立たない） | 6.0 の注記に GPNVM の読み方・`download` が GPNVM1 を立てること・OpenOCD を使わないことを追加。§11 に「ERASE の後に書いたのに動かない = GPNVM1」の行を追加 |
| A5 | `INSTALL.md` §3 の 6.0 の表の下「Which end of the row is pin 7 was not recorded」 | 古い（R177 で SD カード側が 1 番と確定、`tools/fwbackup/README_ja.md` は更新済み） | 直した |
| A6 | `mcu/netgd/.cargo/config.toml` の runner `--chip ATSAM3U4E` | `cargo run` は 4E 向け | コードは変えず、本書 §1-4 と `INSTALL.md` §6（Confirm it worked）の 6.0 の注記で「6.0 では `cargo run` を使わない」 |
| A7 | `fpga/pinfind` のファーム（`pinfind` 機能） | 安全装置（`fpga_off`）が PA25 = nCONFIG を前提にしている。違えば FPGA を止められない（README §0-1 の 2 にも書かれている） | その前に動かす `census` を追加（§2-4）。N4・N6 と合わせて確かめる |
| A8 | NetGD `board-v60` の起動 | PA12（UART のバナー）・PA21（PCK0）・SMC・PB18（失敗時）を、6.0 で確かめずに駆動する | 本書 §1-4・§2-5・§3-3 で「段階 2 の結果を見てから」とした。コードの変更はしていない（ピン探しの結果で `UserPeripherals` を直すときに一緒に扱う） |
| A9 | `docs/boards/gdemu60/README.md` のチェックリスト | 「セキュリティビットが立っているか（SWD で CHIPID が読めるか）」が未完のまま | R177 と判断の記録を書き、本書へのリンクを追加 |
| A10 | `docs/research/user-tasks.html` 作業 3、`tools/netgd-fwupd/README_ja.md`（30・183 行）、`docs/design/g1-bitstream-storage.md` H8 | CHIPID が読める・`fwbackup dump` が取れる前提の書き方 | 変更なし（記録として残す。H8 の「純正のビットストリームの大きさ」は N+1 のクロック数で代わりに分かる） |
| A11 | `INSTALL.md` 冒頭・§6・§7 の 6.0 の注記、`tools/fwbackup/README_ja.md` の「6.0 の現状」、PLAN | 「バックアップを取ってから」 | **ブランチ `docs/locked-6.0`（`e19508ef`、未取り込み）が対応済み**。このブランチは同じ行に触れていない（取り込みの順番はどちらが先でもよい見込み） |

コードの中で「バックアップがある」ことを前提にしているものは見つからなかった。`tools/fwbackup` は読み取り専用で、`write` を拒否する（ERASE の後の GPNVM の操作は素の probe-rs で行う。§1-3）。
NetGD のコードは GPNVM を読まない・書かない（`bootcore::fcr` は SGPB・CGPB・GGPB を作れない。`resident-app-contract.md` L3）。

---

## 出典

- Atmel/Microchip SAM3U データシート 6430（§6.7 ERASE、§8.2.3.4 ロック領域、§8.2.3.5 セキュリティビット、§8.2.3.10 GPNVM、§8.2.4 起動、§20.4.3 EEFC のコマンド、表 4-1 ピン配置）。本リポジトリでの引用：`docs/design/fw-bootloader.md`、`docs/research/web-prep-user-tasks.md` A・E。
- probe-rs 0.32.0（crates.io のソース）`probe-rs/targets/SAM3U.yaml`：`ATSAM3U2E`（NVM 0x80000〜0xA0000、`atsam3u_128`・`atsam3u_gpnvm`）。`atsam3u_128` の命令列をこの PC で逆アセンブルし、UnInit（機能 2）が GPNVM1 を立てて GPNVM2 を消すこと、ProgramPage が CLB → EWP を出すことを確認。
- `docs/research/ag16k-config.md`（AGM の DDF256 マニュアル・AG10K/16K 設計ガイドの要約）。
- 本リポジトリのコード：`mcu/netgd/src/cyclone.rs`（`pinmap`）、`mcu/embassy-atsam3/src/{lib,smc,hsmci,uart}.rs`、`mcu/netgd/src/main.rs`（`FPGA debug`）、`mcu/bootcore/src/fcr.rs`、`mcu/netgd/src/census.rs`・`common/pincensus`（このブランチ）。
