# 実機作業（オーナー作業 3 件）の事前 Web 調査

調査日: 2026-11-20 / 担当: Sonnet 5.5
対象: (1) gdprobe を BBA 経由で純正 GDEMU 6.0 に対して実行 / (2) 6.0 基板のヘッダ導通確認 / (3) Pico debugprobe + probe-rs で CHIPID 読み出し

前回までの調査（`gdemu6-and-parity.md`、`web-findings-2026-11.md`）で出典付きにした項目は再調査していない。ここは「作業の前に知っておくと得な情報・警告」だけを集めた。

信頼度の書き方: **一次資料** = ソースコード・データシート・公式文書を直接読んだ。**二次資料** = 他人の解説・検索要約・フォーラム。**推測** = 上から私が導いたもの。

---

## 0. 結論（先に）

1. **probe-rs は SAM3U をそのまま対応している**（`SAM3U.yaml`、変種 `ATSAM3U2E` あり、フラッシュ 0x80000〜0xA0000・RAM 16 KB）。6.0 基板では **`--chip ATSAM3U2E`** を使う。`INSTALL.md` は 4E 用の例（`ATSAM3U4E`）なので、6.0 では置き換えが必要。（一次）
2. **CHIPID_CIDR は 0x400E0740 で正しい。** ATSAM3U2E の期待値は **0x281A0761**（EXID は 0x400E0744 で 0）。4E は 0x28100961。リビジョンが違えば下位が変わる可能性があるので、`0x281A....` なら 2E と判断する。（一次、ローカルのデータシート表 28-1）
3. **セキュリティビットが立っているときに probe-rs が何と表示するかは、一次資料が見つからなかった。** `INSTALL.md` §6 に書いてある「`The target is not responding / device is locked`」は出典のない例示の可能性が高い。「CHIPID が読めない = ロック」と決めつけず、**`probe-rs info --verbose` で DP（デバッグポート）が見えるか**を切り分けに使う（§C-4）。
4. **ERASE は「電源が入っていて、ピンが 220 ms 以上 High」のときだけ動く。** 電源の入っていない基板への導通テスタは安全（データシート一次資料）。ただし**電源投入中に ERASE（137 番）の隣の TST（138 番）・SHDN（136 番）をショートさせるのは危険**。導通確認は**必ず電源を完全に切り、電源ケーブルを抜いて**から。
5. **gdprobe の CD_READ の byte1 の既定 `0x24` は妥当。** Flycast の解読（一次）では 0x20 / 0x24 / 0x28 はどれも 2048 バイト、`0x10`（other ビット）が 2352 バイト。Linux の `gdrom.c` も実機の GD-ROM 向けに 0x20 を使う。（一次）
6. **openMenu の `gdemu_sdk.c` は、`cdrom_init` もシスコールも使わず、G1 レジスタを直接叩いて `52 xx` を送っている**（一次）。つまり「GDEMU から起動した KOS/ホームブリューが ATAPI を直接送る」実績はある。ただしタイムアウトなし・エラーレジスタ未読なので、gdprobe 側のタイムアウトとエラー読み取りは正しい方針。
7. **6.0 クローンの解析・分解記事・ピン配置は見つからなかった。** Dreamcast-Talk の 6.0 スレ（t=19395）は今回も 403 で本文を読めない。クローンが SAM3U をロックしているかを書いた資料も無い（二次情報として「保護を外すとチップが消える」という伝聞が 1 件あるだけ）。
8. **オープンな他プロジェクト**: OpenGDEMU の上流（cepawiel/OpenGDEMU、Rust + Amaranth + Quartus、Cyclone II 前提）と、別系統の **iceGDROM（zeldin、iCE40 + RISC-V、SD から GD-ROM をエミュレート）**。6.0（AG16K）を対象にした公開プロジェクトは見つからなかった。

---

## A. GDEMU 6.0 / クローン

### A-1 6.0 の分解記事・ピン配置・ヘッダの説明は見つからない

- **答え**: 検索で出たのは Dreamcast-Talk の「GDEMU 6.0 ?」スレだけで、中身は「V6 は AGM チップ、旧基板とクローンは Altera Cyclone」という会話と、販売文面とされる「RISC-V 248 MHz」という記述（実物の SAM3U2E + AG16K と食い違う。`gdemu6-and-parity.md` A-2(b) と同じ）。SWD / UART / JTAG のヘッダがどれかを書いた資料は無い。
- **出典**: https://www.dreamcast-talk.com/forum/viewtopic.php?t=19395 （検索要約のみ。本文は 403）
- **信頼度**: 二次資料（要約）
- **影響**: タスク 2 は完全に自力。**v5.5 / v5.15b の `docs/PCB.md` のヘッダ構成を手がかりに仮説を立てて導通で確かめる**のが一番早い（A-4）。

### A-2 クローンのチップとファーム更新の評判

- **答え**:
  - 5.15b クローンは「SAM3U2E（本家と同じ）、旧クローンは SAM3U4E」というフォーラム投稿がある（二次、2016 年ごろ）。今回の基板の U1（SAM3U2E）は 6.0 でも同じファミリーという意味で一致する。
  - クローンは「ファームを少し改造して動かしているので、本家の更新を当てるとブリックする」という警告と、「ブリックした報告は無い」という反論が混在。5.15b クローンで更新を試したが何も起きなかったという報告もある。販売ページ（ZedLabz）は「クローンは将来の更新不可」とする（`gdemu6-and-parity.md` に既出）。
  - 「クローンのマイコンの保護ビットを外したら読めるようになったが、チップが消えた」という伝聞が 1 件（二次）。データシートの動作と一致する（C-4）。
  - Dreamcast-Talk に「Reflashing Original Firmware on Clone GDEMU」というスレ（t=14980）がある。**本文は 403 で読めていない。** ここに SWD で書き戻した人の手順があるかもしれない。
- **出典**: https://dreamcast-talk.com/forum/viewtopic.php?t=13021&start=30 、https://dreamcast-talk.com/forum/viewtopic.php?t=14980 、https://gbatemp.net/threads/can-the-firmware-on-the-gdemu-5-15b-clone-be-updated.542892/ （いずれも検索要約のみ。403 で本文取得失敗）
- **信頼度**: 二次資料
- **影響**: タスク 3 の結果が「ロックされている」でも、**純正ファームの吸い出しは不可能**という前提を変えない（吸い出し済みの人がいる証拠が無い）。ユーザーのブラウザで t=14980 を読んで貼ってもらう価値あり（§まだ分からないこと）。

### A-3 SAM3U のセキュリティビットをクローンが立てているか

- **答え**: **資料なし。** 本家（Deunan）の純正が立てているという話も、クローンが立てていないという話も見つからなかった。前回の `INSTALL.md` §6 の「stock firmware sets the security bit」は、v5.5 / v5.15b を実際にいじった人の経験談（上流の作者）に基づく可能性があるが、**6.0（AG16K 版）で同じとは限らない。**
- **信頼度**: 推測（根拠となる資料なし）
- **影響**: タスク 3 の目的そのもの（「ロックされているか」を知る）。結果が「読めた」なら大発見なので、**純正ファームを壊さない前提を守ったまま SWD のみ（読み出しだけ）で終える**こと。

### A-4 6.0 のヘッダの仮説（v5.5 の `docs/PCB.md` から）

v5.5 のヘッダ（`docs/PCB.md`、上流作者の実測）:

| v5.5 ヘッダ | ピン数 | 内容 |
|---|---|---|
| H1 | 7 | 3V3 / TDI / TDO(TRACESWO) / **TMS=SWDIO** / **TCK=SWCLK** / NRST / GND |
| H2 | 3 | GND / PA29 / PA31 |
| H3 | 5 | 3.3V / PC28 / PC29 / PC30 / PC31 |
| H4 | 3 | PA11(RX) / PA12(TX) / GND（5.15b クローンでは撤去されている） |

6.0 の目視（`docs/boards/gdemu60/README.md`）: 5 ピン（上辺）、3 ピン（MCU の上）、8 ピン（SD スロット横の縦列）、2 ピン（左）。

- **仮説（推測）**:
  - 5 ピン = H3 相当（3.3V + PC28〜PC31）の可能性。
  - 3 ピン = H2（GND/PA29/PA31）または H4（PA11/PA12/GND）。
  - 8 ピン = SWD/JTAG を含むデバッグ用（H1 の 7 ピン + 1）の可能性。
  - 2 ピン = 電源か LED/ボタン。
- **導通確認の対象ピンを増やす提案**（LQFP-144 の番号。データシート表 4-1、一次）:

| 信号 | ピン | 用途 |
|---|---|---|
| TDI | 1 | JTAG 系ヘッダ判別 |
| TDO/TRACESWO | 4 | 同上 |
| TMS/SWDIO | **7** | SWD |
| TCK/SWCLK | **9** | SWD |
| NRST | **11** | リセット |
| PA29 / PA31 | 46 / 48 | v5.5 H2 相当の確認 |
| PC28 / PC29 / PC30 / PC31 | 69 / 70 / 71 / 72 | v5.5 H3 相当の確認 |
| PA11 / PA12 | **133 / 134** | UART（H4 相当） |
| FWUP / SHDN | 135 / 136 | 参考（ERASE の隣） |
| **ERASE** | **137** | 消去 |
| TST | 138 | **触らない**（FFPI モード用） |
| NRSTB | 141 | 参考 |
| JTAGSEL | 142 | JTAG/SWD の選択。内蔵プルダウンで Low が通常 |
| PA23 / PA24 / PA25 | 103 / 105 / 106 | FPGA コンフィグ線（v5.5） |
| PB1 / PB2 | 55 / 57 | v5.5 での用途（`docs/PCB.md`） |

  - 出典: ローカルのデータシート（Atmel-6430G、表 4-1 144-lead LQFP）。ファイル `.../scratchpad/sam3u.txt` の 636〜712 行。
  - **信頼度**: 一次資料（ピン番号）。ヘッダとの対応は推測。
- **影響（タスク 2）**: GND は基板のシールド缶・USB の外側・コネクタの金属などで確保する。**SWDIO(7) と SWCLK(9) が同じヘッダに出ているはず**なので、まず 7 番ピンから全ヘッダの全ピンに当たって見つけ、続けて 9・11 を確認する。見つかったら **隣のピンとの短絡（SWDIO と SWCLK が並んでいる）に注意**。

---

## B. AGM AG16K のコンフィグ方式（既出の補足）

- **答え**:
  - AGM の公式データシートは、**コンフィグデータが SPI フラッシュ（パッケージ内蔵または外付け）に保存され、電源投入後に FPGA → MCU の順で読み込まれる**と書く（前回と同じ記述の再確認）。MCU 用ファイルと FPGA 用ファイルの 2 つが 1 本の SPI フラッシュに入る。SPI フラッシュ IF は 1/2/4 ビットの NOR フラッシュをサポート。
  - **MCU（SAM3U）からのパッシブシリアルでの書き込みについては、データシートに何も書いていない。** 前回の結論「SAM3U が FPGA を流し込むか内蔵フラッシュで起動するかは実機で決まる」は変わらない。
  - **GDEMU 6.0 の AG16K を解析した公開資料は見つからなかった**（`rodinia`、`iceGDROM` 以外）。
- **出典**: https://www.agm-micro.com/upload/userfiles/files/AG16KPSoC_DS.pdf 、https://github.com/pablomarx/rodinia
- **信頼度**: 一次資料（データシート）
- **影響（タスク 2）**: PA23/PA24/PA25（103/105/106）が **FPGA の基板上のどのパッドにも繋がらず**、AG16K の近くにも出ていなければ「MCU から流し込む方式ではない」可能性が高い。逆に BGA の周辺ビアにつながるなら流し込み方式。**BGA の下に入るので、テスターでは近くのビア/抵抗まで追うだけにする**。結果が出たら `docs/boards/gdemu60/README.md` に追記。

### 他の公開プロジェクト（A の「その他のオープンファーム」）

| プロジェクト | 内容 | 6.0 との関係 |
|---|---|---|
| cepawiel/OpenGDEMU（上流） | Rust ファーム + Amaranth + Quartus。「書くと純正に戻せない」と警告 | Cyclone II 前提。AG16K への言及なし |
| zeldin/iceGDROM | iCE40-HX8K + RISC-V ソフトコアで GD-ROM をエミュレート、SD から読む、GPL-3.0 | 別系統。G1/SPI の実装例としてソースが参考になる |
| mc.pp.se/dc/gdromemu.html | 上の元になった Marcus Comstedt の解説 | AVR + FPGA。プロトコルの詳細は書いていない |

- 出典: https://github.com/cepawiel/OpenGDEMU 、https://github.com/zeldin/iceGDROM 、https://mc.pp.se/dc/gdromemu.html
- 信頼度: 一次資料（READMEを直接読んだ）
- **影響（タスク 1）**: iceGDROM のソース（`rv32/source/`）に SPI コマンドの応答実装がある。純正 GDEMU の記録と食い違いが出たとき、「独立した別実装」として突き合わせに使える（未読）。

---

## C. SAM3U の SWD アクセス

### C-1 probe-rs の対応とチップ名

- **答え**: probe-rs の `probe-rs/targets/SAM3U.yaml` に **ATSAM3U1C / 1E / 2C / 2E / 4C / 4E** がある。2C/2E・4C/4E のフラッシュは 0x80000〜0xA0000（128 KB）、2E の RAM は 16 KB（0x20000000）。アルゴリズム `atsam3u_128`、GPNVM 用の `atsam3u_gpnvm` もあり。追加は v0.12.0（PR #833、2021-09-29）。**ARM のアクセスポートは AP0 のみ。** リセットシーケンスやセキュリティの記述はファイル内に無い。
- **出典**: https://raw.githubusercontent.com/probe-rs/probe-rs/master/probe-rs/targets/SAM3U.yaml 、https://github.com/probe-rs/probe-rs/pull/833
- **信頼度**: 一次資料
- **影響（タスク 3）**: コマンドは次のとおり。
  ```
  probe-rs read --chip ATSAM3U2E --protocol swd b32 0x400e0740 1
  ```
  `INSTALL.md` の `ATSAM3U4E` は 6.0 では **`ATSAM3U2E`** に読み替える。期待値が `0x281A0761`（4E の `0x28100961` ではない）であることもページに明記する。読み出しコマンドは `--chip` の指定を忘れると動かない（`INSTALL.md` の注記どおり）。

### C-2 CHIPID の番地と値

- **答え**: `CHIPID_CIDR` は **0x400E0740**（メモリマップとレジスタ定義の両方で確認）。`CHIPID_EXID` は 0x400E0744。値（表 28-1、Rev A）:

| 品名 | CHIPID_CIDR |
|---|---|
| AT91SAM3U4C | 0x28000961 |
| AT91SAM3U2C | 0x280A0761 |
| AT91SAM3U1C | 0x28090561 |
| AT91SAM3U4E | 0x28100961 |
| **AT91SAM3U2E** | **0x281A0761** |
| AT91SAM3U1E | 0x28190561 |

  レジスタの内訳は下位から VERSION、EPROC、NVPSIZ、NVPSIZ2、SRAMSIZ、ARCH、NVPTYP、EXT（28.3.1）。**VERSION フィールド（下位 5 ビット）は Rev が違うと変わる**ので、0x281A07xx であれば 2E と見てよい（推測）。
- **出典**: ローカルのデータシート `sam3u.txt`（20810〜20850 行）、https://www.keil.com/dd/docs/datashts/atmel/sam3u/doc6430.pdf
- **信頼度**: 一次資料
- **影響（タスク 3）**: 読めた値が `0x281A0761` ならチップは 2E の Rev 1 で、ファームの `sam3u2e` feature と一致。**`0x2810....` なら 4E**（前回の目視と矛盾するので写真を再確認）。

### C-3 JTAG/SWD 系のピンとデバッグポート（データシート）

- **答え**:
  - **SWJ-DP は既定で JTAG-DP が有効**。SWD に切り替えるには専用の JTAG→SWD シーケンスを TMS/TCK に送る。probe-rs / OpenOCD が自動でやる。
  - **JTAGSEL（142 番）が High だと JTAG バウンダリスキャンが選ばれる。** 内蔵 15 kΩ プルダウンで未接続なら Low。基板上でどこかにつながっていて High になっていると SWD が使えない。バウンダリスキャンに入る条件は FWUP/NRST/NRSTB/JTAGSEL が High で TST が Low、電源投入時。
  - BSDL 上の JTAG ID は `05B2_A03F`（これは SWD/JTAG デバッグ時の ARM の ID とは別物。SWD 時は「JTAG 標準でない chip ID」を返すとデータシートが書いている）。
  - TST ピンは FFPI（高速フラッシュプログラミング）用。内蔵プルダウン。
- **出典**: ローカルのデータシート 6.3 / 6.4 / 13.5.7 節
- **信頼度**: 一次資料
- **影響（タスク 2）**: **JTAGSEL（142）と TST（138）が GND に落ちているか（または未接続）を導通で見ておく**と、SWD が通らないときの原因切り分けに使える。

### C-4 セキュリティビットが立っているときの挙動

- **答え**:
  - データシート（一次）: セキュリティ有効時は「ICE（デバッグ）インターフェースと FFPI 経由の **Flash・SRAM・コアレジスタ・内蔵ペリフェラル**へのアクセスはすべて禁止」。解除は **ERASE ピンのみ**（全消去と同時）。ERASE の High は **200 ms より長く**（データシートの別箇所は「220 ms 超」）。
  - **CHIPID レジスタも「内蔵ペリフェラル」なので、ロック時は読めない**（推測。ただしデータシートの文言から自然に導ける）。
  - probe-rs が実際に何と表示するかの**一次資料は無い。** 近い例: nRF52 の保護状態では probe-rs が「AP ... is not a memory AP」を出した（Ferrous Systems の教材）。SAM 系（SAMD）は OpenOCD の保守者が「保護されたチップとは通信できない」と述べる。SEGGER のフォーラムは「SAM 系は接続時に自動で解除できることが多いが、デバッグを恒久的に無効にされた場合は不可」と述べる。**SAM3U で同じ表示になる保証は無い。**
  - ある ATSAM3S2B の利用者は「ERASE を 3.3 V に繋いだが何も起きなかった」と報告（単発）。原因は不明（保持時間や電源投入順の可能性）。
- **出典**: ローカルのデータシート 8.2.3.5 節 / 6.7 節、https://rust-exercises.ferrous-systems.com/latest/book/nrf52/troubleshoot-windows-probe-rs 、https://pyocd.io/docs/security.html 、https://forum.segger.com/ （SAM3S 関連スレ。検索要約のみ）
- **信頼度**: データシートの部分は一次、probe-rs の表示は推測
- **影響（タスク 3）**: 結果の読み方を次のように分けるとよい。

  | 観察 | 解釈 |
  |---|---|
  | `b32 0x400e0740 1` が `400e0740: 281a0761` | 読めた = ロックされていない |
  | `probe-rs info --verbose` で SWD の DP（`DPv1` ...）が見えるが、`read` はエラー | **ロックの可能性が高い**（DP は生きていて AP 越しのアクセスだけが拒否される） |
  | `info --verbose` で DP も見えない | 配線・電源・JTAGSEL・ピン違い。ロックとは言えない |
  | `probe-rs list` にプローブが出ない | プローブ側（C-5） |

  エラー文は**全文をそのまま記録して Claude に渡す**。`INSTALL.md` §6 の「device is locked」という文言は、実際の出力ではなく例示の疑いがあるので、そのまま期待しない。

### C-5 debugprobe ≥ 2.2.0 の要件と Windows の注意

- **答え**:
  - debugprobe のリリース: 2.0.0（2024-03）で picoprobe 名称を廃止、**2.1.0（2024-11）で SWD/nRESET を非アクティブ時にハイインピーダンス化**、**2.2.3（2025 年 7 月）で SWCLK の分周の丸め誤差を修正**（SWD クロック速度に影響）。CMSIS-DAP v2 / WinUSB / probe-rs についてはリリースノートに記述が無い。
  - 現行の debugprobe は **CMSIS-DAP v2（バルク）**で、**MS OS 2.0 記述子に WinUSB の互換 ID を持つ**（`usb_descriptors.c`）。→ Windows 10 以降なら WinUSB が自動で当たるはず。v1（HID）は同時には出ない（ビルド時に選択）。
  - probe-rs の公式ドキュメント: **CMSIS-DAP プローブは Windows でドライバのインストール不要**。Zadig は「nusb 系の別種のプローブ（FTDI、J-Link など）」向けで、「公式ドライバが消える」と警告する。
  - probe-rs の CMSIS-DAP 実装の既定 SWD 速度は 1 MHz。
  - probe-rs の CMSIS-DAP は、**パイプライン返信を落とすファームでは 1 リクエストずつ送る**（遅くなるだけ）。「firmware is outdated」の文言は probe-rs 本体のこのファイルには無い（`INSTALL.md` の記述は別の場所から来ているか、版による）。
- **出典**: https://github.com/raspberrypi/debugprobe/releases 、https://raw.githubusercontent.com/raspberrypi/debugprobe/master/src/usb_descriptors.c 、https://probe.rs/docs/getting-started/probe-setup/ 、https://raw.githubusercontent.com/probe-rs/probe-rs/master/probe-rs/src/probe/cmsisdap/mod.rs
- **信頼度**: 一次資料
- **影響（タスク 3）**:
  - **Zadig は原則使わない。** もし `probe-rs list` に出ないときの最後の手段として、Zadig で **debugprobe の「CMSIS-DAP v2 インターフェース」だけ** に WinUSB を当てる（`INSTALL.md` に記載があれば合わせる）。誤って他のドライバを差し替えない。
  - 2.2.3 以降が望ましい（SWCLK の速度が正しくなる）。
  - 長い配線で通信が不安定なら `--speed 100`（kHz）などで速度を落とす（probe-rs の `--speed` オプション。実在は `probe-rs read --help` で確認）。
  - **Pico 側の電源（USB）と Dreamcast 側の電源（本体）の GND を先に接続**してから SWDIO/SWCLK をつなぐ。`INSTALL.md` §4 の手順（配線 → Pico を PC に → Dreamcast 電源 ON）に従う。

### C-6 読み出し時にコアを止めるか

- **答え**: `probe-rs read` が実行中のコアを止めるかは確認できなかった。ARM のデバッグ接続では通常 `C_DEBUGEN` を立てるが、停止（halt）はしないはずだが、**確証は無い**。
- **信頼度**: 推測
- **影響（タスク 3）**: `--connect-under-reset` は**付けない**（`INSTALL.md` も同じ方針）。NRST は配線しない。読み出しは「GDEMU を Dreamcast から電源投入して通常起動した後」に行い、SD に何かを書き込む操作は一切しない。読み出し後に GDEMU の動作が普通（メニューが出る）か確認する。

---

## D. dcload-ip と GDEMU、KOS からの ATAPI 直接アクセス

### D-1 GDEMU からの dcload-ip の起動

- **答え**: dcload-ip は CD-R からの起動が基本だが、BIOS 経由（DreamDash など）や SD 経由でも使われる。**GDEMU 向けのディスクイメージの作り方を書いた公式資料は見つからなかった**（`dreamcast.wiki` にも記述なし）。ユーザーの `make_dcload_disc.ps1`（既存ノウハウ）に頼る。dcload-ip は BBA（HIT-0400）と LAN アダプタ（HIT-0300）を**1 つのバイナリで**対応。
- **出典**: https://dreamcast.wiki/Dcload-ip 、https://github.com/KallistiOS/dcload-ip （README 経由）
- **信頼度**: 二次資料（検索と要約）
- **影響（タスク 1）**: BBA を使う想定で問題なし。LAN アダプタは使わない。

### D-2 dcload-ip の既知の注意点（README）

- **答え**:
  - ステータスが「link change...」のまま進まないときはケーブル不良。`dc-tool -r` のあとにも出る。
  - 「NO ETHERNET ADAPTER DETECTED!」（赤画面）= 初期化失敗。
  - 例外が起きると水色の画面で 15 秒（既定）止まり、dcload-ip に戻る。
  - **プログラム終了後・リセット後の挙動は文書化されていない。**
  - **G1 / GD-ROM / DMA / GDEMU への言及なし。** dcload-ip 自身が G1 を使うかは不明（ネットワーク経由のコンソール・ファイル I/O が主）。
  - KOS 側（`init.c`）: dcload を検出すると `fs_dcload_init` が走り、IP 版では `net_init` の前にデバッグ出力を無効化して、dcload のネット初期化が成功すれば再有効化する。**dcload 下で cdrom を飛ばす処理は見当たらない**。
- **出典**: https://raw.githubusercontent.com/KallistiOS/dcload-ip/master/README.md 、https://raw.githubusercontent.com/KallistiOS/KallistiOS/master/kernel/arch/dreamcast/kernel/init.c
- **信頼度**: 一次資料
- **影響（タスク 1）**: 既存の注意（「ping が返っても dcload が空いているとは限らない。途中で止めたら電源を入れ直す」）に加え、**gdprobe を途中で強制終了したあとは、GDEMU 側の状態（マウント中の番号）も変わっているので、Dreamcast の電源ごと入れ直す**。

### D-3 KOS の cdrom と G1 の競合（重要）

- **答え**:
  - KOS の `cdrom.c` のセクタ読みは **BIOS のシスコール**（`syscall_gdrom_send_command` / `exec_server` / `check_command`）経由。生の G1 レジスタは `cdrom_init`（バス保護の解除）と `unlock_dma_memory`（DMA 保護の解除）でしか触らない。
  - `cdrom_init` は `_g1_ata_sem` を取り、BIOS の 0xe6ff マジックを確認し、`syscall_gdrom_init()` を呼び、ASIC の GD-DMA 割り込みハンドラと **vblank ハンドラ**を登録し、最後に `cdrom_reinit()`（`thd_poll` を使うので**スレッドとタイマーが動いていること**が前提）。
  - **`fs_iso9660` は vblank ハンドラで `cdrom_get_status()` を呼び、ディスク交換を検出すると `cdrom_reinit()` を走らせる。** `fs_iso9660_init` 自体は `cdrom_init` を呼ばない。
  - KOS 側の `cdrom_init` が呼ばれるタイミング（既定のスタートアップ）は今回確認できなかった。
- **出典**: https://raw.githubusercontent.com/KallistiOS/KallistiOS/master/kernel/arch/dreamcast/hardware/cdrom.c 、https://raw.githubusercontent.com/KallistiOS/KallistiOS/master/kernel/arch/dreamcast/fs/fs_iso9660.c
- **信頼度**: 一次資料（上記の挙動）。init での呼び出し元は未確認。
- **影響（タスク 1）**: gdprobe の `gdraw.c` は `g1_ata_mutex_lock` で `cdrom.c` と同じセマフォを取る設計（コメントどおり）。**ただし `52 82` でイメージを切り替えた直後、KOS の vblank ハンドラ（`fs_iso9660`）が「ディスク交換」を検出して `cdrom_reinit()` をバックグラウンドで走らせる可能性がある。** そうなると `CD_CMD_INIT` が G1 に流れ、プローブの直接コマンドとぶつかる。対策案:
  - プローブ起動時に `fs_iso9660` を初期化しない（KOS の `KOS_INIT_FLAGS` から `INIT_FS_ISO9660` を外す）か、
  - 切替後に 2〜3 秒待ってから（vblank 側の再初期化が終わってから）発行する。
  - 切替後の最初のコマンドは `TEST UNIT READY` を何度か送って READY を待つ（openMenu が 20 ms 間隔で最大 500 回待つのと同じ。`web-findings-2026-11.md` 1-2）。
  - **どちらにするかは、ユーザーの KOS フォークの `init.c` / `cdrom.c` を見て決めるべき**（今回の調査では確認できず）。

### D-4 openMenu の `gdemu_sdk.c`（実績のある直接アクセス）

- **答え**: `52` コマンドは **G1 のレジスタ（0xA05F7xxx）を直接叩く**。手順は「BSY/DRQ が落ちるのを待つ → `A0`（PACKET）を COMMAND に書く → DRQ を待つ → STATUS を 1 回ダミー読み → 12 バイト CDB を 16 ビットで 6 回 DATA に書く → `EXT_INT_STAT`（0xA05F6904）の bit0 をポーリング → STATUS を読む」。**データを返すコマンドでは先に FEATURES に 0 を書き、`LBA_MID/HIGH` からバイト数を読む。** コードはタイムアウト無し、エラーレジスタ未読、バッファ上限なし（`send_packet_data_command` の型）。
- **出典**: https://github.com/sbstnc/openmenu/blob/HEAD/src/openmenu/src/backend/gdemu_sdk.c
- **信頼度**: 一次資料
- **影響（タスク 1）**: `gdraw.c` のレジスタ番地とシーケンスは一致している。差異: gdraw は BYTE_LIMIT を 0x0800 に設定して PACKET を発行、タイムアウトあり（良い）。**ダミー読み（STATUS を DRQ 後に 1 回）が gdraw にもあるか確認**（openMenu は付けている。理由はコメントに無いが、割り込み/ステータスのクリアの意味と思われる）。

### D-5 CD_READ（0x30）の CDB byte1

- **答え**:
  - Flycast（`gdromv3.h/.cpp`）は byte 1 を次のビットフィールドで解釈する（LSB から）: bit0 = prmtype（0 = FAD、1 = MSF）、bit3..1 = expdtype（期待セクタ種別）、bit4 = other（同期/ECC/EDC）、bit5 = data（ユーザーデータ）、bit6 = subh（サブヘッダ）、bit7 = head（ヘッダ）。ビット位置は**ビットフィールドの宣言順から導いた**（コンパイラの並び順に依存するので推測を含む。KOS の定数と整合する）。
  - Flycast のセクタサイズ決定:
    - head=1 かつ subh=1 かつ data=1 かつ expdtype=3 かつ other=0（= byte1 `0xE6`）→ **2340**
    - other=1 または expdtype=1（CD-DA）→ **2352**
    - それ以外 → **2048**（`0x20`、`0x24`、`0x28` はここ）。未対応の組み合わせは警告を出すだけで 2048。
    - 2336 バイトは出てこない。
  - 数値の対応（byte1）: `0x20` = ユーザーデータのみ（期待種別を限定しない）、`0x24` = Mode 1 + データ、`0x28` = Mode 2 Form 1 + データ、`0x10` = other ビットのみ。
  - KOS の `cdrom_change_datatype` が使う `sector_part` は `CDROM_READ_WHOLE_SECTOR = 0x1000`、`CDROM_READ_DATA_AREA = 0x2000`。**これは byte1 の上位ニブル（0x10 / 0x20）を 8 ビット左にずらした値と一致する**（偶然でなければ BIOS がそのまま byte1 に変換している。推測）。既定の `sector_size` は 2048（データ領域）と 2352（全セクタ）。
  - Linux の `gdrom.c` は実機向けに **`cmd[0] = 0x30`、`cmd[1] = 0x20`** を使い、1 セクタ 2048 バイトで DMA 読みする。
  - 0x30 の 3 バイト長のセクタ数は `b[8..10]`（0x31 は 2 バイトで `b[6..7]`）、開始 FAD は `b[2..4]`。→ `gdprobe` の `cdb_read` と一致。
  - 参考: REQ_MODE は byte2 = オフセット、byte4 = 長さ。GET_SCD は byte1 の下位ニブル = フォーマット、byte3-4 = 確保長。これも `cdb_simple` / `cdb_be16` と一致。
- **出典**: https://raw.githubusercontent.com/flyinghead/flycast/master/core/hw/gdrom/gdromv3.cpp 、https://raw.githubusercontent.com/flyinghead/flycast/master/core/hw/gdrom/gdromv3.h 、https://raw.githubusercontent.com/KallistiOS/KallistiOS/master/kernel/arch/dreamcast/include/dc/syscalls.h 、https://raw.githubusercontent.com/torvalds/linux/master/drivers/cdrom/gdrom.c
- **信頼度**: 一次資料（Flycast のコードと Linux ドライバ）。ビット位置と KOS 定数の対応は推測。**純正 GDEMU が何を返すかは Flycast の解釈とは別問題**（Flycast は仕様側の解釈、GDEMU は FPGA 実装）。
- **影響（タスク 1）**: `cdread_flags` の既定 `0x24` は妥当。`gdprobe/README_ja.md` の「BIOS が送る値の記憶に基づく推測」を、「Flycast / Linux の解読と整合」に更新してよい。**プローブで 0x20 / 0x24 / 0x28 / 0x10 を試して、GDEMU がそれぞれ何バイトを返すか（`total / count`）を記録する**と、NetGD が合わせるべき動作（byte1 の解釈）が分かる。このため `probes.c` に「同じ FAD を byte1 だけ変えて 4 回読む」項目を追加する価値がある（読み取りのみ、安全）。注意: 0x10 は「CD-DA 以外のデータトラックで 2352 を返すか」を見る。**エラー（ERR）になっても記録を続けて次へ進む**こと。

### D-6 DreamShell / GDmenu / `gdemu_sdk`

- **答え**: DreamShell のソースは今回取得していない。openMenu の `gdemu_sdk.c`（上記）が「ホームブリューから GDEMU に `52` を送る」実績の一次資料で、`gdemu_control.c` は上位の制御。GDmenu / DreamShell の GDEMU ローダーの細部は未調査。
- **信頼度**: 二次（未調査）
- **影響**: 優先度は低い。gdprobe の `0x52 0x82` の形式は openMenu と一致していることが確認済み。

---

## E. LQFP-144 の導通確認と ERASE のリスク

### E-1 ERASE のしくみ（データシート一次資料）

- ERASE ピンは**内蔵 15 kΩ プルダウン**。未接続で通常動作。
- **100 ms 未満の High は無視**（SCLK でデバウンス）。**220 ms 超**の High でフラッシュ再初期化。「ERASE のアサートは常に 200 ms より長く」とも書かれる。
- 低消費電力モードでもアサートすればチップを起動して消去する。
- 入力レベルの基準電源は **VDDBU**（表 3-1）。
- ERASE はロックビットとセキュリティビット（GPNVM0）、GPNVM1 などを消す。キャリブレーションビットとユニーク ID には影響しない。
- 出典: ローカルのデータシート 6.7 節、8.2.3.5 節（1348〜1362 行、1720〜1745 行）、表 3-1。Microchip の新しい資料にも「電源投入時のグリッチによる誤消去を避けるため最小アサート時間がある」との記載（https://onlinedocs.microchip.com/oxy/GUID-174CB4CE-C435-49E8-BF01-3B8ABC11F056-en-US-8/GUID-B4538C69-39FC-4537-903A-07D2004586E4.html の検索要約）。SAM3S 系の文書は「ERASE を I/O として使う場合は起動時 Low にする」と書く（二次）。
- **信頼度**: 一次資料

### E-2 リスク評価

| 状況 | リスク | 理由 |
|---|---|---|
| **電源オフの基板**にテスターの導通モードを当てる | **ほぼ無し** | チップに電源（VDDBU/VDDCORE）が無く、消去動作を実行できない。テスターの出力は数 mA 以下・時間も短い。100 ms 未満の High は無視される |
| 電源オンの基板で ERASE をテスターで触る | 避ける | 電源が入っていれば 220 ms 以上触れていれば消える。プローブ先端が滑って**隣（136 SHDN、138 TST）**に触れる危険もある |
| 電源オンで TST（138）を High にする | 避ける | FFPI（高速フラッシュ書き込み）モードに入る可能性（電源投入時に High の場合） |
| 導通テスタが基板を逆給電する | 極小（推測） | ESD 保護ダイオード経由で微小電流が流れて部分的に電源が入る可能性は理論上あるが、消去を完遂する電圧・時間には届かない |

- **信頼度**: データシートからの推論（推測を含む）。**ERASE を電源オフの基板に導通テスタで当てて消えた事例は見つからなかった。**
- **影響（タスク 2）**: 手順に次を入れる。
  1. **Dreamcast から外す（または電源ケーブルを抜き）、SD カードも抜いて**、基板を完全に無電源にする。コンデンサの放電に少し待つ。
  2. 導通モード（またはダイオードモードではなく**抵抗レンジ**）で、**黒プローブを GND に固定**し、赤で探る。
  3. ERASE パッドは「パッド ↔ 137 番ピン」の**導通だけ**を確認し、触れる時間は短くする（誤って電源が入っても 100 ms 未満なら無視される）。
  4. 基板の電源を入れる前に、**ERASE へ配線したままにしない**（`INSTALL.md` §6 と同じ）。

### E-3 LQFP-144 の実用的なコツ

- **答え**（一般的なヒント。検索結果から。信頼度は二次〜推測）:
  - 0.5 mm ピッチでは通常のテスターのプローブ先端は太すぎる。**細いプローブ・ピンセット・フック型のミニプローブ**を使う。
  - **ピンに直接当てるより、ピンから近い抵抗・ビア・テストパッド・ヘッダ側から確認する**。ピンとの導通を確認したいときは、ピンの根元（ランド側）に先端を軽く当てる。
  - 黒プローブは GND に固定し、赤プローブを動かす。ピンの上を**引きずらない**。
  - 事前に拡大鏡で隣接ピンのハンダブリッジの有無を確認する。
  - 手元に実体顕微鏡が無ければ、**スマートフォンのマクロ撮影 + 拡大**でピンの数え間違いを防ぐ。1 番ピンは丸い刻印やドット。**4 辺とも 36 ピン**で、1 番の隣は 144 番（反時計回り）。
  - ダイオードモードは ESD ダイオードを順方向に電流を流す。電源オフなら破損しないが、解釈が難しくなるので**抵抗/導通レンジ**を使う（推測）。
  - ESD 対策にリストストラップか、作業前に接地金属に触れる。
- **出典**: https://hackaday.com/2024/08/14/hacker-tactic-pimp-your-probes 、https://resources.pcb.cadence.com/blog/2024-ic-continuity-test
- **影響（タスク 2）**: ピン数えの間違いが一番ありがちな失敗。**SWDIO(7)・SWCLK(9)・NRST(11) は 1 番ピンの辺の近くで 2 個おき**（TMS=7、PB29=8、TCK=9、PB28=10、NRST=11）。MCU 側で 7・9・11 が 1 つおきに並ぶ（間の 8・10 は PB29・PB28 で別信号）ことを、ピン数えの検算に使う。

---

## ページ（実機作業ガイド）に反映すべき点

### 全体

1. **「純正ファームを壊さない」原則に、次の一文を足す**: 「タスク 2 は基板を無電源にして行う。タスク 3 は読み出し（CHIPID）だけ。ERASE パッドには、どのタスクでも配線しない」。
2. **作業の順序を固定**: タスク 1（Dreamcast 側のみ）→ タスク 2（無電源）→ タスク 3（SWD 読み出し）。タスク 3 は 2 の結果（SWD が出ているヘッダ）が前提。

### タスク 1（gdprobe）

3. 実行前の確認に「**KOS フォークで `fs_iso9660` の vblank が切替直後に `cdrom_reinit` を走らせないか**（`KOS_INIT_FLAGS` で ISO9660 を外すか、切替後に数秒待つ）」を追加。（D-3）
4. `cdread_flags` の説明を更新: 「既定 0x24 = Mode 1 のデータ（2048 バイト）。Flycast と Linux のドライバの解釈に整合。0x20（Linux が使う値）、0x28 を併せて試すと GDEMU の byte1 の解釈が分かる」。（D-5）
5. 試験項目に「同じ FAD を byte1 = 0x20 / 0x24 / 0x28 / 0x10 で読み、返ったバイト数（total / count）を記録」を足すことを提案（`probes.c` の追加はコード側の作業）。**0x10 でエラーになっても続行**。
6. 「gdprobe が途中で落ちたら Dreamcast の電源を入れ直す」を明記（D-2）。
7. dcload-ip の起動画面で「link change...」のままの場合はケーブル/BBA を確認（D-2）。

### タスク 2（導通確認）

8. **対象ピンの表を差し替え**（A-4 の表）。SWDIO 7 / SWCLK 9 / NRST 11 に加えて、TDI 1・TDO 4・JTAGSEL 142・TST 138（触らない）・PA29 46・PA31 48・PC28〜31 69〜72 を追加。
9. 「**ヘッダは v5.5 では H1(7 ピン SWD)、H2(3)、H3(5)、H4(3 UART)。6.0 は 8/5/3/2 ピンなので、7 番ピンから全ヘッダの全ピンに当てて SWDIO を探し、同じヘッダ内で 9・11 を確認する**」という探索手順を追加。
10. **無電源での作業**を最初の手順に明記（E-2）。SD カードも抜く。
11. **ERASE パッドは「パッド ↔ 137 番」だけを短時間で確認し、他のピン（136・138）に当てない**。
12. FPGA 側: PA23/24/25（103/105/106）から「FPGA 周辺のビア・パッド」まで追えたかを記録（B）。

### タスク 3（probe-rs で CHIPID）

13. コマンドの `--chip` を **`ATSAM3U2E`** にする（`ATSAM3U4E` ではない）。期待値 **`400e0740: 281a0761`**。（C-1、C-2）
14. 結果の読み方の表（C-4）をページに入れる。**エラーは全文を記録する**。`info --verbose` の出力も取る（DP が見えるかで切り分け）。
15. **`--connect-under-reset` を付けない・NRST を配線しない・ERASE に触れない**。
16. Windows: **Zadig は使わない**（debugprobe v2 は WinUSB の記述子を持つので自動で入る）。`probe-rs list` に出ないときだけ、USB の接続を変えてみてから Zadig を検討。debugprobe は **2.2.3 以降**を推奨（C-5）。
17. 手順は `INSTALL.md` §4 の順序（配線を先に → Pico を PC に → Dreamcast の電源）に従う。GND を最初に接続。
18. 結果が「ロックされている」でも、**その場で ERASE をしない**（純正ファームの保存方針を別途決めるまで）。

### 文書（リポジトリ内）の修正案（ページ作成とは別。Claude 側の作業）

19. `INSTALL.md` §6 の「`Error: The target is not responding / device is locked`」は出典のない例示の可能性があるので、「実際の表示は未確認」と注記する。
20. `tools/gdprobe/README_ja.md` の `cdread_flags` 説明を「推測」から「Flycast・Linux と整合」に更新。

---

## まだ分からないこと

1. **6.0 のどのヘッダが SWD/UART か**（実物の導通でのみ決まる）。
2. **6.0 の純正ファームがセキュリティビットを立てているか。** 「ロック時に probe-rs が出す正確なエラー文」も未確認。実機の出力が最初の一次資料になる。
3. **AG16K のコンフィグ方式**（MCU から流し込むか、パッケージ内蔵の SPI フラッシュか）。データシートは PS のピン名もタイミングも書いていない。導通でも決めきれない可能性がある。
4. **Dreamcast-Talk の 3 スレ（t=19395 / t=14980 / t=13021）の本文。** WebFetch は 403。ユーザーのブラウザで読んで貼ってもらう（特に t=14980「Reflashing Original Firmware on Clone GDEMU」は SWD の手がかりを含む可能性）。
5. **KOS の `cdrom_init` が既定のスタートアップで呼ばれるか**、および vblank ハンドラと G1 の競合（D-3）。ユーザーの KOS フォークのソースで確認が必要。
6. **GDEMU が CD_READ の byte1 の違いにどう応答するか**（Flycast の解釈とは別）。タスク 1 の結果で分かる。
7. **`probe-rs read` がターゲットのコアを止めるか**（C-6）。ソース（`probe-rs-tools` の `read` と `Session` の attach）を読めば確定できる。
8. **debugprobe の WinUSB が Windows 11 で自動インストールされるか**の実機報告。設計上は自動のはず（記述子から）。
9. **導通テスタの出力電圧と、無電源の MCU への影響**の定量データ（メーカー資料）。理屈は上記のとおりだが、実測値の裏づけは無い。
10. DreamShell / GDmenu の GDEMU 制御のソース（今回は未調査）。
