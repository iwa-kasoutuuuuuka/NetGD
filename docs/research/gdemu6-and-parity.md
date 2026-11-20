# GDEMU 6.0 ハードウェア調査（G1）と本家機能パリティ（G3）

調査日: 2026-11-20 / 担当: Sonnet 5.5（G1-1・G3-1）

信頼度の表記: **[A]** 出典 URL つきの事実、**[B]** フォーラム等の伝聞、**[C]** 不明、**[実物]** ユーザーが自分の 6.0 基板で直接読んだ事実（`docs/boards/gdemu60/README.md`）。
本家公式サイト（gdemu.wordpress.com）には **6.0 という版は載っていない**（公式はハード v5 / v5.1、最新ファーム 5.20.5。後述 [A]）。「6.0」はクローン業者の名乗りか、非公式の改版と見るのが安全。

---

# Part A — G1: GDEMU 6.0 基板

## A-1. 実物で確定していること [実物]

| 記号 | 刻印 | 意味 |
|------|------|------|
| U1 | Atmel ATSAM3U2E（日付 2527） | MCU。**SAM3U4E ではなく 2E**（フラッシュ 128 KB 単一プレーン） |
| U2 | AGM AG16KDDF256（PP6669 1921, FBGA-256） | FPGA。**Cyclone II ではなく AGM** |
| Y1 / Y2 | 12 MHz / 11.2896 MHz | v5.5 と同じ 2 本 |
| U3 | 74HC04D | インバータ。水晶発振回路か（仮説） |
| U4 | 1117-1.2 | 1.2 V LDO（FPGA コア用と推定） |
| Q1 | 6201E | SOT-23 電源系 IC か |
| ヘッダ | 5 / 3 / 8 / 2 ピン、シルクなし | どれが SWD / UART かは未特定 |

## A-2. 公開情報から分かったこと

### (a) 出典つき事実 [A]

1. **公式の GDEMU にハード「6.0」は無い。** 公式ページはハード v5 / v5.1 のみ言及、ファーム最新は 5.20.5（2020 年以前出荷の機は 5.20.0 未満だと 2 段階更新が必要）。
   https://gdemu.wordpress.com/firmware/updating-gdemu/ 、https://gdemu.wordpress.com/installation/gdemu-installation/
2. **AG16K の公式データシート（AG16K PSoC Rev1.0）**: https://www.agm-micro.com/upload/userfiles/files/AG16KPSoC_DS.pdf
   - LE 16,624 / 埋め込みメモリ 504 Kbit（M9K）/ 18x18 乗算器 56 / **PLL 4 基** / グローバルクロック 20 / I/O バンク 8。
   - コア電圧 **1.2 V**（1.15〜1.25 V）。I/O は 3.3 / 2.5 / 1.8 / 1.5 V LVCMOS。PLL 用 VCCA・VCCD_PLL も別電源。→ 基板の U4（1.2 V LDO）と整合。
   - **型番規則**: `AG16K` + `[SD|DD|空]` + `[E|F]` + ピン数。**SD = SDRAM 内蔵、DD = DDR RAM 内蔵、空 = FPGA のみ**、F = FBGA（1.0 mm ピッチ）。
     表 1-2 に `AG16KDDF256M`（FBGA-256、17x17 mm、最大 169 I/O）がある。
     → **実物の `AG16KDDF256` は「FPGA + ARM Cortex コア（最大 200 MHz、SRAM 128 KB）+ DDR 128 Mbit を 1 パッケージにした PSoC 系」である可能性が高い。**
     ただしデータシートは 16K LE の FPGA 部と ARM 部を併記しており、**6.0 がその ARM コアを使っているかは不明** [C]。
   - コンフィグ: 機能欄に「JTAG and AS, PS」（AS = アクティブシリアル、PS = パッシブシリアル）。一方 6.1 節は「コンフィグデータは SPI フラッシュに置き、電源投入で FPGA→MCU の順に読む」と書く。
     **PS のピン名・タイミング・ビットストリーム大きさは本文に無い**。ビットストリーム圧縮（実時間展開）に対応。
   - 同じ AG16K の製品ページ（FPGA 単体 AG16KF256）はコンフィグを「JTAG と SPI」とだけ書き、LQFP-144/176・FBGA-256、ピン表 `AG16KF256_Pinout.xls` を挙げる。 http://www.agmsemi.com/program_details_agmfpgaag16kf256.html
3. **SAM3U ファミリー**（Microchip/Atmel doc6430）: SAM3U4E は 2×128 KB フラッシュ（2 プレーン）、SAM3U2E は 128 KB 単一プレーン。SRAM は 4E が SRAM0 32 K + SRAM1 16 K + NFC 4 K（計 52 K）、2E は 16 K + 16 K + 4 K（計 36 K）。両者とも LQFP/BGA-144、96 MHz、HSMCI 8 bit 幅。
   https://www.keil.com/dd/docs/datashts/atmel/sam3u/doc6430.pdf （検索結果の抜粋で確認。巨大 PDF のため全文取得は失敗。上記の 2E の数値は本リポジトリ `mcu/netgd/link/sam3u2e.x` の記述とも一致）
4. **SD カード・ファイルシステム（本家）**: SD 512MB〜2GB、SDHC 4〜32GB 推奨、SDXC 非対応、FAT16/FAT32、**exFAT 非対応**、先頭パーティションのみ使用、クラスタは大きく（32 KB 典型）。 https://gdemu.wordpress.com/details/
5. **Supra（AGM の IDE）**: フォーラムで「Quartus 13.1 と Supra-2023.02.b0 の組み合わせで動いた」「ネイティブモードの合成は Yosys」との報告。 https://www.eevblog.com/forum/fpga/agm-cplds-fpgas/ （投稿者の主張。後述 [B] の扱い）

### (b) フォーラム伝聞 [B]

- Dreamcast-Talk「GDEMU 6.0 ?」スレ https://www.dreamcast-talk.com/forum/viewtopic.php?t=19395 （**このスレは WebFetch が 403 で本文を読めていない**。検索結果の要約のみ）:
  - 販売文面とされる記述: 6.0 は旧チップをやめ「AGM の RISC-V ヘテロジニアスチップ（248 MHz コア + 2K CPLD）」を使う。→ **実物（SAM3U2E + AG16K）と食い違う。** RISC-V 248 MHz は AG32VF 系の宣伝文句であり、販売文面は 6.0 の内部を正確に言っていない可能性が高い（信頼しない）。
  - 「旧基板とクローンは Altera Cyclone、v6 は AGM」というコメントは実物と一致する。
  - 「Deunan は新リビジョンを出していないと思う。更新されたクローンでは？」「旧基板と同じに動くはず、純正ファームを使うから」— いずれも憶測。
- 販売ページ: クローン（v5.15 / v5.20 名乗り）は「将来ファーム更新不可」とする記述あり（ZedLabz、購入者レビュー）。 https://www.zedlabz.com/products/gdemu-sd-memory-card-adapter-for-sega-dreamcast-optical-drive-simulator-pcb-board-v5-15-replacement-zedlabz
- eevblog: 「AGM の FPGA は MAX II / MAX V / Cyclone IV のクローンに見える」（投稿者の印象）。「Supra は Verilog のみ、VHDL 不可」と「Quartus 24.1 スクリプトで VHDL も可」が食い違う。「PLL / DAC を Quartus が最適化で消して AGM の PNR が見つけられない」という報告あり。

### (c) 不明 [C]

- 6.0 基板で SWD / UART / ERASE がどのヘッダ・パッドか。
- U2 の PS コンフィグピンが MCU のどのピンに配線されているか（FPGA コンフィグを MCU から流し込む設計か、外付け SPI フラッシュか）。
- U2 内蔵の ARM コアを純正 6.0 が使っているか。
- 純正ファームのフラッシュがセキュリティロックされているか（SAM3U は ERASE ピンで必ず解除可能。ただし 2E の ERASE 位置は 4E と同じ 144 ピンパッケージなので同じはず）。
- SD スロットの配線（HSMCI 4 bit のままか）。
- AG16K のビットストリームサイズ、PLL 属性、PS 書き込み速度上限。
- 本家（Deunan）との関係。

## A-3. v5.5 / v5.15b と 6.0 の差分表

| 項目 | v5.5 / v5.15b クローン（`docs/PCB.md`） | 6.0（実物） | 状態 |
|------|----------------------------------------|-------------|------|
| MCU | SAM3U4E-AU（256 KB 2 プレーン、SRAM 52 K） | **SAM3U2E**（128 KB 単一、SRAM 36 K） | [実物] |
| FPGA | EP2C5T144C8N（Cyclone II、LQFP-144、1.2 V コア） | **AG16KDDF256**（AGM、FBGA-256、1.2 V コア。PSoC 系の可能性） | [実物]＋[A] |
| 水晶 | 11.2896 MHz（FPGA に直結）+ 12 MHz（MCU） | 11.2896 MHz + 12 MHz + 74HC04D | [実物]。74HC04 は発振回路（FPGA が水晶直結でなくなった）か |
| SD | HSMCI 4 bit、カード検出 PA1 | 不明 | [C] |
| FPGA コンフィグ | MCU の PA23–25・PB1–2 から PS（DATA0/DCLK/nCONFIG/CONF_DONE/nSTATUS） | 不明。外付け SPI フラッシュの有無を確認要 | [C] |
| MCU–FPGA バス | SMC（NCS3）16 bit + IRQ PA22 + 48 MHz PWM PA28 + リセット PA30 | 不明。BGA で導通追跡が難しい | [C] |
| ヘッダ | H1 SWD 7 ピン、H2、H3、H4 UART 3 ピン（v5.15b は H4 なし） | 5 / 3 / 8 / 2 ピン、シルクなし | [実物]（機能は未特定） |
| 電源 | 不明 | 1117-1.2 + 6201E | [実物] |
| ファーム更新 | クローンは SD の `GDEMU.FW` 方式の可能性（未確認） | 不明 | [C] |

## A-4. 6.0 基板でユーザーが確認すべきこと

写真は上面・下面・拡大（Y2/U3/U4 付近）に加えて、**下面の SD スロット周りとコンフィグ周りを高解像度で**。

1. **FPGA の近くに 8 ピン SOIC のフラッシュ（W25Q/25Q 系、刻印 25Qxx など）があるか。** 無ければ MCU 側から PS で流し込む設計、有れば FPGA は自分でフラッシュから起動している（→ MCU から書き換えるには SPI の配線が要る）。最重要。
2. **U1 周りの導通**（テスターのブザー、基板に電源を入れずに）: SWDIO（SAM3U2E の TMS/SWDIO ピン）・SWCLK・NRST・GND から 5 / 3 / 8 / 2 ピンのどのヘッダにつながるか。UART は PA11（RX）・PA12（TX）、v5.5 の H4 と同じ並びかを見る。
3. **ERASE ピンのパッド**: SAM3U の ERASE ピン（LQFP-144 の該当ピン番号はデータシートのピン表で確認）から出ている試験パッドの位置。
4. **U1 から FPGA 方向への配線**: SMC 系（PB6〜PB16, PB19, PB23, PB25〜PB31, PC0〜PC6, PC15, PC17）が 5.5 と同じ MCU ピンで使われているか、MCU 側ピンから先の VIA を追う。MCU 側が LQFP なので MCU ピン→近傍パターンは追いやすい。
5. **SD スロットの接点と MCU PA/PB ピンの導通**（HSMCI: PA3〜PA8 付近。データシートで確認）、カード検出スイッチ、ライトプロテクト。
6. **前面ボタンが PA20 か**、LED が PB18 か。
7. **U2 の刻印末尾**（`M` の有無、`AG16KDDF256` か `AG16KDDF256M` か）。PSoC 系の判定に使う。1921 は日付コード（2019 年第 21 週）で、基板が 2025 年製の MCU と一緒なので在庫 FPGA の可能性。
8. **SWD で SAM3U2E の CHIPID を読む**（`probe-rs read --chip ATSAM3U4E --protocol swd b32 0x400e0740 1`、INSTALL.md §6 と同じ。2E は CHIPID が 4E と異なるはずなので、読めた値でパーツを確定する）。読めなければセキュリティロックの可能性。
9. **ロックされていた場合の破壊的手順（ERASE）の前に、純正ファームの吸い出しは不可能**（ロック時は読み出せない）。先に出荷時の SD やファーム更新ファイルの有無を確認し、**元に戻せなくなることを承知のうえで**進める。

## A-5. 差分ごとの移植影響

| 差分 | 影響 | 作業量 |
|------|------|--------|
| **MCU: SAM3U4E → SAM3U2E** | 同ファミリー・同 LQFP-144 でペリフェラルも同一。**HAL の書き換えは不要。** リポジトリに `sam3u2e` feature・`link/sam3u2e.x` あり（NFC SRAM に CDDA リング、`MAX_IMAGES` 制限）。ただしフラッシュ 128 KB の単一プレーンに**コードとビットストリームを同居**させる必要があり、`DEFMT_LOG=off` が前提（`build.rs:102`）。さらに「実行中のプレーンを消さない」制約が効く：`fwupdater` は SRAM1 から動く設計（`build.rs build_updater`）だが、単一プレーンでは書き換え対象と実行元が同じプレーンになるので、**RAM 実行なら問題ないが、4E の「プレーン 1 に新ビットストリーム」型の段階更新は使えない**。EFC は 2E でも 1 つ。ERASE ピン・GPNVM の扱いは同じ。 | 小〜中（ビルドは通る見込み。ビットストリームの置き場所が最大論点） |
| **FPGA: Cyclone II → AG16K** | **新しい合成・配置配線ターゲット。** (1) Amaranth の Verilog（`fpga/amaranth/out/OpenGDEMUCore.v`）は流用できる見込み。(2) Quartus の `.qsf` / `ip/*`（PLL）は使えない。AGM の Supra（または Quartus 13.1 + Supra の併用）で PNR。ピン割り当てを AG16KDDF256（BGA）用にゼロから作る。(3) PLL は AG16K の PLL 属性（M/N/C 1〜512、出力 5 本）に合わせて再生成。**PLL がツールチェーンで落ちるという eevblog 報告に注意**。(4) 現行 FPGA が必要とするクロックは PA28 の 48 MHz（MCU 供給）+ 水晶 11.2896 MHz。AG16K でも専用クロック入力ピンから PLL に入れる必要がある（データシート: 汎用 I/O は PLL に入らない）。(5) **コンフィグ方式が最大の未確定要素**。PS が使えて MCU が流し込めるなら現行の `cyclone.rs`（PS ローダ）を**タイミングと信号極性の確認後に流用できる可能性**はあるが、AG16K の PS 仕様は未公開／未確認。外付け SPI フラッシュ方式なら、MCU はビットストリームを SPI フラッシュに書く書込み機能が新たに必要。 | **大**（ビットストリーム生成とコンフィグ経路が未確定。オープンツールは無い） |
| FPGA の ARM コア（PSoC 系の場合） | 本家 6.0 が使っていても、NetGD は MCU = SAM3U2E で十分。**使わなければ無視でよい**が、電源投入時に FPGA がデフォルト状態で何をするか（コア未使用ならハイZ）を確認する。 | 小 |
| 74HC04D / 水晶回路 | Y2 の発振用と思われる。FPGA クロック入力に正しい信号が来るかの確認のみ。 | 小 |
| ピン配置のみの差 | **ボード機能として吸収**（Cargo feature `board-v55` / `board-v60`）。UART・SWD ヘッダが違う場合は INSTALL に別手順を足すだけ。SMC/HSMCI ピンが同じなら変更なし。 | 小 |

### AG16K ツールチェーンの現状

- Supra は AGM 公式配布（Baidu クラウド等）で、**オープンツールチェーンは確認できていない**。eevblog では「Supra のネイティブモードは内部で Yosys を使う」という報告があるだけで、AGM のビットストリーム形式の公開情報は無い（gerbay が「AGM の暗号化ファイルをデコードする」ツールを公開したとのみ言及）。
- 公式 FAQ 的な一次情報は見つからなかった。Quartus から読み込めるか（Quartus プロジェクトを Supra が読む「Quartus モード」）は報告ベース。
- **Quartus 13.0.1（本リポジトリのコンテナ）と Supra の組み合わせが動くかは未検証**。Supra は Windows / Linux ネイティブで配布されるので、現行の podman ベースの Quartus イメージとは別環境になる。

---

# Part B — G3: 機能パリティ

## B-1. 本家 GDEMU の公開機能（出典つき）

| 項目 | 内容 | 出典 |
|------|------|------|
| 対応機 | VA1 のみ。VA2 不可、VA0 は損傷の恐れ | https://gdemu.wordpress.com/details/gdemu-details/ |
| イメージ形式 | GDI、CDI、CCD+IMG+SUB、MDS+MDF、ISO（自作ソフト用） | 同上 |
| フォルダ | 2 桁（01〜99）、3 桁（〜999）、4 桁（〜9999）。8.3 ファイル名。主ファイルは `disc.*` | 同上 |
| 既知の非互換 | PC ドライブで「スワップ法」ダンプした新しい GDI は track3 以降の開始アドレスの計算が違い動かない | 同上 |
| SD | SD 512MB〜2GB、SDHC 4〜32GB、SDXC 不可、FAT16/FAT32、exFAT 不可、先頭パーティションのみ | https://gdemu.wordpress.com/details/ |
| 選択 | カード挿入で 01 が選ばれる。ボタンで次へ、最後の次は 01 に戻る。01 を空にすると自動起動しない。使い始めは 02 から | https://gdemu.wordpress.com/operation/gdemu-operation/ |
| 複数ディスク | 同一カードに全部入れてボタンで切替、またはカード交換 | 同上 |
| 動作中の抜き差し | 電源オンのままカード交換可 | 同上 |
| LED | 電源投入で 1 回点滅して消灯。点灯・点滅継続は内部エラー（電源入れ直し） | 同上 |
| GDEMU.INI（ルート） | `open_time` 150〜5000 ms（既定 500）、`detect_time` 150〜5000 ms（既定 250）、`image_tests` 0/1（既定 1）、`high_speed` 0/1（既定 0、実験的）、`reset_goto` 0〜9999、`read_limit` 600〜1250 KiB/s / 0 / -1（自動） | 同上 |
| ファーム更新 | `GDEMU.FW` をルートに置き、ライトプロテクト on でカード挿入→電源。LED の点滅パターンで完了判定。最新 5.20.5 | https://gdemu.wordpress.com/firmware/updating-gdemu/ |
| ファーム履歴 | 5.13.0 で CCD 解析改善・スクランブル CCD・MCN/ISRC サブチャンネル走査、5.20.0 で SoA / RE:CV の自動タイミング・速度制限 INI | 同上 |
| メニュー | **GDMenu / openMenu は本家機能ではなく外部ソフト**。サードパーティの Card Manager が SD を組み立て、**フォルダ 01 にメニューを置く** | https://github.com/sonik-br/GDMENUCardManager 、https://www.segasaturnshiro.com/2026/01/21/breathing-new-life-into-gdemu-with-openmenu-virtual-folder-bundle/ （フォルダ 01 の説明は検索要約。[B]） |
| Card Manager 形式 | GDI、CDI、MDS、CCD、CHD、CUE/BIN（変換して GDEMU 形式にする）。`name.txt` を各フォルダに作る | 上の GitHub |
| IGR / 地域・VGA パッチ | **公式ページには記載なし**。openMenu 側の機能として「fast boot」「BIOS に戻る」等はあるが、GDEMU ファーム機能ではない | https://retrorgb.com/breathing-new-life-into-the-gdemu-with-the-openmenu-virtual-folder-bundle.html |

### ユーザーの実機ノウハウ（実機確認済み(本家)）

出典: `iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA` の KNOWHOW_ja.md §6（ユーザーのメモ。https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA ）。

- SD は **MBR パーティションあり** の FAT32。パーティションなし（スーパーフロッピー）では**何も起動しなかった**。
- フォルダは `01` から連番、`01` は GDmenu。`GDEMU.ini` はルート。
- openMenu 用の管理ツールで GDmenu のカードを保存すると `01` が置き換わる。

## B-2. 機能パリティ表

優先度: **P0**=基板が動かない・メニューが使えない、**P1**=日常使用、**P2**=あると良い。

| 機能 | 本家 | NetGD 現状（ファイル:関数 の根拠） | 差分/必要作業 | 優先度 |
|------|------|------------------------------------|--------------|--------|
| SD の MBR + FAT32 | **実機確認済み(本家)**: MBR 必須、スーパーフロッピー不可。FAT16/32、exFAT 不可、先頭パーティション | `fat.rs:probe` は MBR の先頭パーティション、または LBA0 が BPB ならカード全体を使う（スーパーフロッピーも受理）。FAT32 以外は拒否（`fat.rs:probe` の `root_entries != 0 || spf16 != 0` で FAT16 不可）。`ide.rs:size_disk_from_mbr` は MBR の署名が無ければ CSD のサイズへフォールバック。マウント自体は embedded-sdmmc の `open_volume(VolumeIdx(0))`（`gdi.rs:load_by_name`） | 受け入れ側が本家より緩いのは問題なし。**FAT16 非対応**は ≤2GB カード用に本家と差（`fat.rs` の高速経路。embedded-sdmmc 経由の `gdi.rs` 側は FAT16 を読める可能性があるが未検証）。**テスト項目**: MBR 無しカードで起動しないことは本家の挙動と無関係（ユーザー環境の注意喚起として INSTALL に既記）。 | P1（FAT16） |
| 番号付きフォルダ（01, 02, …） | **実機確認済み(本家)**: 01 から連番、01 = GDmenu。2/3/4 桁対応 | 名前ベースのみ。`gdi.rs:load_by_name` はフォルダ名一致（大文字小文字無視）で 1 階層ずつ降りる。`gdi.rs:load_default` は `SELECTED_NAME` → `GDEMU.CFG` の `image=` の順で、**どちらも無ければ空のまま**（`gdi.rs:583` の warn、`GdiError::NoGdiFile`）。番号を解釈する処理は無い | **GDMenu/openMenu カードが動かない根本原因の候補。** 無設定時の既定を「フォルダ `01`（または 2/3/4 桁の最小番号）」にする。番号フォルダの列挙順の定義（桁数の混在）を決める。`INSTALL.md:358` の「Without this file the firmware mounts whatever it finds first」は `gdi.rs:load_default` の挙動（空で起動）と食い違うので直す | **P0** |
| 設定ファイル | **実機確認済み(本家)**: ルートに `GDEMU.ini`。キーは上表 | `gdi.rs:configured_name` は **`GDEMU.CFG`** の `image=` だけを読む（`CONFIG_NAME`、`gdi.rs:404`）。`GDEMU.ini` は読まない。本家キー（open_time 等）は全て未実装 | `GDEMU.ini` の読み込みを追加（`reset_goto` / `read_limit` / `open_time` / `detect_time` / `image_tests`）。`GDEMU.CFG` は NetGD 独自拡張として残すか、`GDEMU.ini` の `[netgd]` セクションに統合（DOS 8.3 名の `GDEMU.INI` は FAT の大文字小文字を無視するので問題なし） | P1 |
| メニュー（GDMenu / openMenu） | 外部ソフト。**実機確認済み(本家)**: 01 に置かれ、GDmenu が選んだ番号を本家ファームが受けてマウントする（受け方は未確認） | 0xE0 `IMAGE_CTL` で**フォルダ名**マウント（`ide.rs:handle_image_ctl`、`SUB_MOUNT_NAME`/`SUB_MOUNT`）。GDMenu / openMenu が本家に送る**コマンド形式は本調査で未確認** [C] | **最優先の調査項目**: openMenu のソース（`gdemu` ブリッジ部分）で、メニューが GDEMU に何を送ってディスクを切り替えるか確認し、NetGD で同じコマンドを受ける。形式が分からないうちは P0 のまま | **P0** |
| イメージ形式 | GDI、CDI、CCD+IMG+SUB、MDS+MDF、ISO | `gdi.rs:find_gdi`（拡張子 GDI の最初の 1 ファイル）と `cdi.rs:find_cdi`。CCD / MDS / ISO は無い | Card Manager は多くの形式を GDI に変換して書くので、**まず GDI/CDI だけで実用上は足りる**。CCD/MDS/ISO は P2。`disc.gdi` という名前の要求は無い（任意の `*.GDI`）で本家より緩い | P2 |
| GDI 解析・スワップ法ダンプ | track3 以降のアドレス計算が違う新しい GDI は非対応（本家も） | `gdi.rs:parse`。LBA 計算はファイル記述通り。既知の非互換には言及なし | 同じ入力に対し同じ結果になるか、スワップ法 GDI を使った比較は未実施。実機で確認 | P2 |
| CDDA | あり（GDI の audio トラック） | `cdda.rs`（`SECTOR_BYTES = 2352`、リングを FPGA に給餌）。`ide.rs:handle_packet` の `SEGA_CD_PLAY`/`SEEK`/`GET_SCD` | 実装済み・実機未検証。**グリッチ測定が必要**（B-4） | P1 |
| ディスク入替・複数ディスク | ボタンで次の番号へ、末尾は 01 に戻る | `button.rs:on_press` → `gdi.rs:next_of_mounted`（`GAME.CFG` の `next` 名のリンク先）→ `ide.rs:request_swap`。**番号の順送りは無い**。GAME.CFG の無いフォルダでは何も起きない（`button.rs`） | 本家と同じ「番号の次、最後は 01」を既定動作にする。`GAME.CFG` の `next` は拡張として残す（複数ディスクのペアリング） | **P0**（01 の GDmenu からの復帰に必要） |
| カード挿入で 01 を選択 | あり | **起動時に 1 回だけ** `sdcard::init`（`main.rs:main_task`）。カード検出 PA1（`docs/PCB.md`）はコード上未使用（`embassy-atsam3` にピン定義のみ） | 活線挿抜（カード交換時の再初期化＋再マウント）を実装 | P1 |
| `reset_goto` | 設定で N 番へ | なし | `GDEMU.ini` 実装時に追加。リセット検出は FPGA の `ide_rstn`（`main.rs` の diag、`bus_state`）が使える可能性 | P2 |
| LED | 1 回点滅して消灯、エラー時は点灯／点滅継続 | `main.rs` の最終ループが **毎秒トグル**（ハートビート、`led.toggle()`）。エラー表示は無い | 本家の意味に合わせる（点滅1回→消灯、エラーは点灯）。ハートビートはデバッグ feature に退避 | P2 |
| カード容量・速度 | SDHC ≤32 GB、`high_speed` は実験的、`read_limit` で速度制限 | `sdcard.rs`: 4 bit・24 MHz（`set_clock_hz(24_000_000)`、CMD6 の高速切替は未実施）。SDHC/SDXC のアドレス指定は対応（`CardCapacity`）。CSD の容量が過少に出るバグは MBR 側で補正（`ide.rs:size_disk_from_mbr`） | 50 MHz（CMD6 高速モード）で本家超えのスループットを狙える。`read_limit` 相当（SoA / RE:CV のタイミング依存ゲーム）は実機の挙動を見て追加 | P1 |
| 起動時のイメージ検査 | `image_tests`（既定 1） | なし（`gdi.rs:parse` が基本的な検査のみ） | 巨大メディア許容の `image_tests=0` 相当は不要に近い。優先度低 | P2 |
| ファーム更新 | SD の `GDEMU.FW` + ライトプロテクト + LED 表示 | 0xF9 ベンダーコマンド（セカンダリ IDE 経由、2 段キー）→ `fwupdate.rs:handle` → RAM 実行の `mcu/fwupdater`（SRAM1） | 本家方式（SD 上のファイル）は未実装。コンソール不要の運用なら SD 方式を追加（SAM3U2E は単一プレーンなので更新は RAM 実行必須。既にそう） | P1 |
| セカンダリ IDE ディスク | なし（本家には無い） | あり（`ide.rs:handle_disk_read`/`handle_disk_write`、G2 の土台）。GD-ROM マウント中は書込み拒否 | NetGD 独自の追加機能。保持 | — |
| MCN / ISRC サブチャンネル | 5.13.0 で対応 | `ide.rs:GET_SCD`（フォーマット 1 は Q サブチャンネルのみ、`ide.rs:3151` 付近）。MCN/ISRC は未確認 [C] | ISRC/MCN を返すゲームが存在するか実機で確認 | P2 |
| IGR / 地域・VGA パッチ | 公式ファームに記載なし | なし | 本家に無い。実装不要（NetGD の独自機能としてなら別途検討） | — |
| exFAT | **非対応** | 非対応（`fat.rs` は FAT32 専用） | 本家と同等。将来 exFAT を対応すれば「より高性能」になるが優先度低 | P2 |
| 同梱 INI 互換カード | Card Manager が生成した `GDEMU.ini` / `name.txt` | `name.txt` は NetGD に読む処理なし（フォルダ名＋`GAME.CFG` の title） | メニューはフォルダの `name.txt` を自分で読む（DC 側）ので、NetGD ファームには不要 | — |

### 差分の要点（上位 5）

1. **番号フォルダ・無設定時の既定が無い**（`gdi.rs:load_default` が空で起動）。GDmenu/openMenu カードそのままで動かない。
2. **メニューからの切替コマンドの互換性が未確認**（0xE0 は NetGD 独自）。
3. **ボタンが番号順送りでない**（`button.rs` は `GAME.CFG` のリンク前提）。
4. **`GDEMU.ini` が未対応**（`GDEMU.CFG` のみ）。`INSTALL.md` の無設定時の記述も実装と食い違う。
5. **カードの活線挿抜と SD 経由のファーム更新が未対応**。

## B-3. 「より高性能」の指標

| 指標 | 内容 | 本家の目安 / 備考 |
|------|------|-------------------|
| 電源オン→メニュー表示 | DC の電源 ON から GDMenu / openMenu のタイトルが出るまで | 本家は 5 秒前後の報告が一般的だが**出典なし。同一カードで本家・NetGD を並べて測る** |
| ゲーム読み込み時間 | メニューで選択→ゲームのタイトル画面まで、およびゲーム内ロード（特定ステージ開始） | 同条件で比較 |
| シーク遅延 | CD_SEEK から最初のセクタが返るまで。`ide.rs` の `CMD_SEEN_AT` / `LAST_CMD_DONE` を使った内部計測が既にある（`ide.rs:912` 付近） | defmt ログで平均・最大を出す |
| 持続読み出しスループット | 連続セクタ読みの KiB/s。SD は 4 bit 24 MHz（`sdcard.rs`）→ 50 MHz で伸びる余地 | `sd-bench` feature（`sdcard.rs:bench_read`）で SD 単体を、DC 側で実測 |
| CDDA の欠け | 音が途切れる回数。ゲーム 2〜3 本のトラックを 30 分通し再生して耳で数える。内部は `cdda.rs` のリング残量の最小値を出力 | 目標 0 回 |
| 互換性 | 一般的な GDI（SoA、RE:CV、Sonic Adventure など）の起動成功率 | 本家には `read_limit` / 自動タイミング補正がある |

**特別な機材なしでの測り方**

- 時間: スマートフォンのスロー動画（240 fps）で DC の電源投入からの画面変化を撮り、フレーム数から計算（±10 ms）。
- 内部レイテンシ: NetGD 側は UART ログ（H4）に `Instant` の差を出す（既存の計装を流用）。本家側は UART が無いので動画計測のみ。
- スループット: DC 上でメモリ読み出しを繰り返す自作ホームブリュー（KallistiOS）で、固定サイズ（例 8 MiB）を CD_READ して `timer` で計測。dcload-ip で PC に結果を出す（ユーザーの BBA 環境）。
- CDDA: 音声を DC のライン出力から PC のマイク入力に取り込み、波形の無音区間を検出（Audacity の「無音検出」）。
- 比較は**同一 SD カード・同一ゲーム・同一ディスク位置**で行い、各条件 5 回以上の中央値を取る。

---

## 参考 URL 一覧

- 公式: https://gdemu.wordpress.com/ （details / operation / installation / firmware 各ページ）
- AG16K データシート: https://www.agm-micro.com/upload/userfiles/files/AG16KPSoC_DS.pdf
- AG16KF256: http://www.agmsemi.com/program_details_agmfpgaag16kf256.html
- Supra 配布ページ: https://www.agm-micro.com/solutions.aspx?lang=&id=77&p=61
- eevblog AGM スレ: https://www.eevblog.com/forum/fpga/agm-cplds-fpgas/
- SAM3U データシート: https://www.keil.com/dd/docs/datashts/atmel/sam3u/doc6430.pdf
- Dreamcast-Talk 6.0 スレ（本文未取得）: https://www.dreamcast-talk.com/forum/viewtopic.php?t=19395
- GDMENUCardManager: https://github.com/sonik-br/GDMENUCardManager
- openMenu Virtual Folder Bundle: https://www.segasaturnshiro.com/2026/01/21/breathing-new-life-into-gdemu-with-openmenu-virtual-folder-bundle/
- ユーザーの BBA ノウハウ: https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA

## 未解決・要追跡

- Dreamcast-Talk の 6.0 スレ本文（403 のため未読）。ブラウザ系ツールで読むか、ユーザーが貼り付ける。
- AG16K の PS コンフィグ仕様・ビットストリームサイズ（データシートのコンフィグ章と `Manual_*` 系資料。Supra 配布物に含まれる可能性）。
- GDMenu / openMenu が本家 GDEMU に送る切替コマンド（openMenu ソースで確認）。
- 本家の EEPROM 的な設定保存の有無（`GDEMU.ini` のみか）。
