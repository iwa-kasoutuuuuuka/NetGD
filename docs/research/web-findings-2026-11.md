# Web 調査メモ（2026-11 追加分）

調査日: 2026-11-20 / 担当: Sonnet 5.5
前回（`gdemu6-and-parity.md`）で出典付きにした項目は再調査していない。ここは「未取得」だった項目を埋める追加分。

信頼度: **一次資料** = ソースコード・公式文書・データシートを直接読んだ。**二次資料** = 他人の解説・検索要約。**推測** = 上から私が導いたもの。

---

## 結論（先に）

1. **GDmenu / openMenu が GDEMU に送る切替コマンドが分かった（一次資料）。** 標準 ATAPI の `0x52` で、非データのパケットコマンド。byte1 でサブコマンドを選ぶ。
   - `52 82 <lo> <hi> ...` : イメージ番号 N（1〜999）をマウント。0 = 既定イメージに戻す。
   - `52 81 <0x55|0x44> ...` : 次 / 前のイメージ。
   - `52 00 ...` : バージョン取得。8 バイトのデータを返す。
   - NetGD は今 `0xE0` しか持たない。**`0x52` を足せば openMenu のカードがそのまま動く見込み（P0）。**
2. **メニューの一覧は SD も ini も直接読まない。** メニュー自身がマウントされたディスク（フォルダ 01）の `/cd/OPENMENU.INI` を読む。`[ITEMS]` の `NN.name` 形式で、**スロット番号 = SD のフォルダ番号**。GDEMU 側に「一覧を返す」コマンドは無い。
3. 切替直後、openMenu は `cdrom_reinit()` を 20 ms 間隔で最大 500 回（約 10 秒）繰り返して READY を待つ。**切替後は NOT READY → MEDIA CHANGE → READY の順で応答する必要がある**（本家の `open_time` 500 ms / `detect_time` 250 ms がこれに当たる）。
4. 本家の ini・ボタン・活線挿抜・ファーム更新は公式ページで裏が取れた（下記 2 章）。**`image_tag` は公式ページに無い。** 実在するか不明なので実装対象から外す。
5. **AG16KDDF256 は FPGA + Cortex コア + 128 Mbit DDR + SPI フラッシュを 1 パッケージに積んだ品**（一次資料）。コンフィグは JTAG / AS / PS の 3 モード。ビットストリームは内蔵 SPI フラッシュに置く設計。6.0 基板がこのフラッシュを使うのか、MCU から PS で流すのかは**不明のまま**。
6. **オープンなツールは実質ある。** `pablomarx/rodinia`（mmicko のフォークあり）が Supra を解読し、Supra 付属の Yosys（`synth_agm`）→ `.vqm` → `af`（P&R）→ `.bin` の流れ、PLL のインスタンス化（`alta_pllx`）、ビットストリームの展開スクリプトまで公開している。ただし**nextpnr 対応は AG1KLPQ48 だけ**。AG16K は `agm-unpack.py` が AG16KSDE176（非圧縮のみ）に対応。
7. KOS の `g1ata` は **KOS 2.1.0 から**（2.0.0 には無い）。IDENTIFY の word 49 bit9（LBA）、word 83 bit10（LBA48）、word 63 bit2（MWDMA mode 2）を見る。**FLUSH CACHE は BSY が落ちるまで待つだけでタイムアウトもエラー確認も無い。**
8. CDI（音声 + データ）のセッション 2 は、Flycast では**CDI 自身の記述子の値**を返す。固定値は仮定しない。NetGD の `boot_area_lba` が 45000 固定なのはこの点で誤り。
9. 11700 と 11702 の食い違いは未解決（資料ごとに違う）。実物の CDI で確認する必要がある。
10. バスのビット反転・タイミングについて、ODE 側の公開資料は**見つからなかった**。

---

## 1. GDmenu / openMenu ↔ GDEMU（G3 差分表 #2）

### 1-1 コマンド形式（一次資料）

出典: openMenu（sbstnc フォーク、megavolt85 版ベース）のソース。
- https://github.com/sbstnc/openmenu/blob/HEAD/src/openmenu/src/backend/gdemu_sdk.c
- https://github.com/sbstnc/openmenu/blob/HEAD/src/openmenu/src/backend/gdemu_sdk.h
- https://github.com/sbstnc/openmenu/blob/HEAD/src/openmenu/src/backend/gdemu_control.c

| 機能 | CDB（12 バイト、残りは 0） | 転送 | 備考 |
|------|---------------------------|------|------|
| バージョン取得 | `52 00 00 ...` | **データあり**（8 バイト） | 応答コメント: `00 00 09 01 00 00 14 05`。`01 09 00 00` が内部ブートローダ版、`05 14 00 00` が FW 版（コメントでは 5.14.0）。**コメントのバイト並びと数値が合わない**（0x14 = 20）。並びは推測にとどめる |
| 番号指定でマウント | `52 82 <N 下位> <N 上位> 00 ...` | なし | N = 0 は「既定イメージに戻す」、1〜999 は番号。16 ビット LE |
| 次 / 前 | `52 81 <0x55 次 | 0x44 前> 00 ...` | なし | |

実装上の振る舞い（`send_packet_command`）:
1. BSY/DRQ が落ちるのを待ち、`0xA0`（PACKET）を書く。FEATURES は触らない（データなしの場合）。
2. DRQ を待って 12 バイトを 16 ビット × 6 で書く。
3. **G1 の割り込み（EXT_INT_STAT bit0）を待つ。** → NetGD は非データ応答でも完了時に INTRQ を上げる必要がある。
4. ALTSTATUS の ERR だけを見て戻る。**エラー時の詳細（REQ_SENSE）は読まない。**

データありの `0x52 0x00`（`send_packet_data_command`）は、LBA_MID / LBA_HIGH のバイトカウントを読んで、その長さだけ 16 ビットで読む。応答が 8 バイトならバイトカウントも 8 にする。

### 1-2 切替後の流れ（一次資料）

`dreamcast_launch_disc()` の順序:
1. `gdemu_set_img_num(slot_num)`
2. `wait_cd_ready()` = `cdrom_reinit()` が `ERR_OK` になるまで 20 ms 間隔、最大 500 回
3. `cdrom_get_status()` でディスク種別（`CD_GDROM` か）を読む。これを `disc_type` としてローダに渡す。
4. `gdmenu_loader` を `arch_exec` で起動（ブート ROM の IP.BIN 経由ではなく、メニューが自分で起動する）。

→ NetGD への含意:
- 切替後、**最初の応答は NOT READY / UNIT ATTENTION（MEDIA CHANGE）でよい**が、10 秒以内に READY になること。`cdrom_reinit` は内部で GET_STATUS / REQ_SENSE / TOC 等を呼ぶので、新イメージの TOC が正しく返ること。
- `cdrom_get_status` の `disc_type` が `CD_GDROM` かどうかで GDI / CDI が区別される。**GDI では GD-ROM（0x80 系）、CDI では CD-ROM XA を返す**。これは既存の `ide.rs` の GET_STATUS と同じ規則で足りるはず。

### 1-3 一覧の取得方法（一次資料）

出典: https://github.com/sbstnc/openmenu/blob/HEAD/src/openmenu_shared/src/backend/gd_list.c と `gd_item.def`
- `PATH_PREFIX "/cd/"`。つまり**メニューが入っているディスク（フォルダ 01 のイメージ）の中**から `OPENMENU.INI` を読む。
- キーは `[ITEMS]` セクションで `NN.name` / `NN.disc` / `NN.vga` / `NN.region` / `NN.version` / `NN.date` / `NN.product`。`NN` の整数が `slot_num` になり、**そのままフォルダ番号として `set_img_num` に渡される**（`atoi(slot_string)`、最大 3 桁）。
- SD カードにも `GDEMU.ini` にも触らない。**カード上の一覧を作るのは PC 側の GDMENUCardManager**（https://github.com/sonik-br/GDMENUCardManager ）で、メニュー用ディスクイメージに `OPENMENU.INI` と画像（`0GDTEX.PVR` など）を焼き込む。
- 本家 GDEMU に「SD のフォルダ一覧を返すコマンド」は無い。

GDmenu（元祖）はクローズドソースで、ソースは見つからなかった。openMenu の SDK ヘッダは megavolt85 による 2019 年版（`File Created: 20th January 2019`）で、**GDmenu の逆解析から出たものと推測**する（二次資料にも確認が無い。推測）。

### 1-4 NetGD への影響

| 項目 | 内容 |
|------|------|
| PLAN | G3-2 の #2（GDmenu / openMenu のコマンド対応）、#1（番号フォルダ、既定 `01`）、#3（ボタン送り） |
| 追加先 | `mcu/netgd/src/ide.rs` のパケットディスパッチ（`OGDM_IMAGE_CTL = 0xE0` の隣）。`0x52` ハンドラを足す |
| 設計 | `52 82`: N = 0 → 既定（`load_default` 相当）、N >= 1 → `gdi.rs` に「番号 N のフォルダ」を探す関数を足してマウント。`52 81`: `0x55` = `next_of_mounted` の番号版、`0x44` = 前。`52 00`: 8 バイトを返す |
| 未決 | 応答の FW 版バイトを何にするか（NetGD 版を名乗る / 5.20.x を名乗る）。openMenu 側は**バージョンで挙動を変えていない**ことをソースで確認済み（`gdemu_get_version` を呼んでいる箇所が見当たらない。呼び出し元の全文検索はしていないので、断定はしない） |
| テスト | `common/discimg` に 12 バイト CDB の解釈（`parse_gdemu_cmd`）を切り出し、ホストテストで固定できる |

---

## 2. 本家 GDEMU の挙動（G3 差分表の裏取り）

出典（すべて公式、一次資料）:
- https://gdemu.wordpress.com/operation/gdemu-operation/
- https://gdemu.wordpress.com/details/gdemu-details/
- https://gdemu.wordpress.com/firmware/updating-gdemu/

### 2-1 `GDEMU.ini`（SD のルート）

| キー | 値 | 意味 |
|------|----|------|
| `open_time` | 150〜5000 ms、既定 500 | イメージ切替後に「蓋が開いている」状態を見せる時間 |
| `detect_time` | 150〜5000 ms、既定 250 | 切替後に「BUSY（ディスク検出中）」を見せる時間 |
| `image_tests` | 0 / 1、既定 1 | 0 で整合性検査を緩める（大きすぎるメディアを許す。動作保証なし） |
| `high_speed` | 0 / 1、既定 0 | 実験的な高速 SD 転送 |
| `reset_goto` | 0〜9999、0 で無効 | システムリセット検出時に N 番へ切り替える（5.8.1 で追加） |
| `read_limit` | 600〜1250 KiB/s / 0 無効 / -1 自動 | 実機に近づけるための読み出し速度制限（5.20.0） |

- **設定はカード検出のたびに読み直される。** カードごとに別設定にできる。**INI が無ければ前の値を保持**する（既定値に戻さない）。
- **`image_tag` は公式ページに無い。** 実在するか確認できなかった（推測: 別の版のクローンか、誤記）。
- ファーム履歴の ini 関連: 5.4.0 で `open_time` / `detect_time`、5.6.0 で `image_tests`、5.13.0 で MCN/ISRC のサブチャンネル走査の無効化、5.20.0 で速度制限。

### 2-2 フォルダ番号付け・ファイル名

- 各イメージは専用フォルダ。**フォルダ名は 01〜99 は 2 桁、100〜999 は 3 桁、1000〜9999 は 4 桁**（公式）。
- 対応形式: **GDI / CDI / CCD+IMG+SUB / MDS+MDF / ISO**（ISO は自作ソフト用）。
- ファイル名は **DOS 8.3 のみ**、長いファイル名と非 ASCII は非対応。**メインのイメージ名は `disc` + 拡張子**（`disc.gdi`、`track01.bin`、`track02.raw`、`track03.bin`）。
- 「swap method」で作られた PC 製 GDI（トラック 3 以降にデータやデジタル音声がある場合）は壊れる。
- → NetGD の `GAME.CFG` とフォルダ名方式は拡張として残し、**番号フォルダ内の `disc.gdi` / `disc.cdi` 探索を既定にする**。ここは `gdi.rs` の改修。

### 2-3 ボタン・カード・リセット

- ボタンで次のイメージへ。**最後の次は 01 に戻る**。短押し / 長押しの区別は公式に記載なし。
- カードを挿すと**イメージ 01 が選ばれる**。起動中の差し替え（活線挿抜）が可能。電源 ON は有カードでも無カードでもよい。
- 01 を自動起動させたくないときは 01 を空にして 02 から入れる。
- 初期の v4/v5 は挿抜時に基板を手で押さえる必要がある（機械的な話）。
- LED: 電源投入で 1 回点滅して消灯。**点灯または起動後の点滅は内部エラー**。復旧は本体の電源再投入。
- リセット: 既定ではイメージを変えない。`reset_goto` > 0 で N 番へ。

### 2-4 SD からのファーム更新（公式手順）

1. ファームを `GDEMU.FW` にリネームし、SD のルートに置く。
2. **SD のライトプロテクトを LOCK にする。**
3. DC の電源を切ったまま挿入、電源 ON。LED が速く点滅する。
4. LED が **0.5 秒点灯 / 0.5 秒消灯**になるまで待つ。
5. 電源 OFF、カードを抜き、ロック解除、**`GDEMU.FW` を削除**（誤更新防止）。
- LED が点灯または消灯のまま、または他のパターンならエラー。
- 2020 年以前の出荷でファームが 5.20.0 未満のものは、先に updater を書く 2 段階更新が必要。

NetGD への含意:
| PLAN | 内容 | 場所 |
|------|------|------|
| G3-2 #5 | 更新ファイル名 `GDEMU.FW`、**ライトプロテクトが立っているときだけ更新する**（誤更新防止）、完了は LED 0.5 秒点滅 | `mcu/netgd/src/fwupdate.rs`、`main.rs` の起動処理 |
| 注意 | **SAM3U2E は 128 KB フラッシュ**で、現行の `0xF9` 方式（RAM 常駐 updater）がそのまま使えるか要確認 | `mcu/fwupdater` のサイズ |
| G3-2 #4 | `GDEMU.ini` は**カード検出のたびに読み直す**。無ければ前の値を保持 | `gdi.rs` の設定読み出し |
| P0 | 切替後の挙動に `open_time` → `detect_time` の順（NOT READY / MEDIA CHANGE）が必要。**これが 1-2 のコマンドと一体** | `ide.rs` の `request_swap` |

SD のライトプロテクト検出が基板上にあるか（SD ソケットの WP ピン）は未確認。G2（SD への書き込み）とも関係するので、実機で WP 配線を見る必要がある。

---

## 3. AGM AG16K FPGA（G1 の本体）

### 3-1 AG16KDDF256 の中身（一次資料）

出典: AGM 公式 データシート AG16K PSoC Rev1.0
https://www.agm-micro.com/upload/userfiles/files/AG16KPSoC_DS.pdf
（ローカルで `pdftotext` して読んだ）

- **「1 つのチップに プロセッサ・FPGA・SDRAM/DDR・ADC・Flash を入れた」** と明記（2.1 節）。DD = DDR 128 Mbit 内蔵（16 ビット幅、最大 400 MHz）、SD = SDRAM 64 Mbit 内蔵、型番 AG16K の後ろが空なら FPGA のみ。
- 表 1-2 に載っている組み合わせ: AG16KSDE176M / AG16KSDE176GM / AG16KSDF256M / **AG16KDDF256M**（FBGA256、17×17 mm、1.0 mm ピッチ、最大 169 I/O）。
- MCU: Cortex（M3 級 ETM の記述あり）、最大 200 MHz、SRAM 128 KB、UART 1、GPIO 24、SPI フラッシュ IF、JTAG / SWD。FPGA 側から AHB マスタ / スレーブで繋がる。
- FPGA: 16,624 LE、504 Kbit、18×18 乗算器 56、**PLL 4 基**、グローバルクロック 20、I/O バンク 8。
- PLL は**入力クロックが同じ辺のクロックピンからのみ**（汎用 I/O からは入れられない）。フィードバックモードは 4 種。
- **コンフィグ（6 章）: 「コンフィグデータは SPI フラッシュに保存される。保存ファイルは MCU 用と FPGA 用の 2 つ。電源投入後、先に FPGA が読まれ、次に MCU 初期化ロジックが指定アドレスから MCU 用を読む」**。コンフィグモードは **JTAG / AS / PS**。圧縮ビットストリーム対応。
- MCU は内部 SPI フラッシュにも、FPGA I/O 経由の外部 SPI フラッシュにもアクセスできる。

**AG16KDDF256（M なし）とデータシートの AG16KDDF256M の違いは不明**（M = 温度等級か別版かは書かれていない。推測）。

### 3-2 含意（推測を含む）

| 論点 | 判断 |
|------|------|
| DD は SoC か | データシート上は **「SoC（ARM + FPGA + DDR + フラッシュ）」**。Y/N は **6.0 基板がこの ARM を使っているか**で、これは未判明。基板に ATSAM3U2E が別にあるので、ARM は**使われていない可能性が高い**（推測） |
| ビットストリームの置き場所 | パッケージ内に SPI フラッシュがあるなら、**FPGA は電源投入で自動的にそのフラッシュから読み込む**。この場合 ATSAM3U2E が FPGA をコンフィグしない（`mcu/netgd` の `fpga.rs` に当たる処理が要らない）。逆に PS で流す設計なら MCU 側の書き込みが要る。**どちらかは基板上の配線（nCONFIG / DCLK / DATA0 / CONF_DONE 相当が MCU に繋がっているか）で判断する。`docs/boards/gdemu60/README.md` の「ヘッダ 5/3/8/2 ピン」を実測する項目に足す** |
| 書き換え | 内蔵フラッシュを書き換えられるなら、**ビットストリームの更新は SD → MCU → FPGA 経由のフラッシュ書き込み**になる。書き込み用の JTAG / AS ピンがパッドとして出ているか要確認 |

この 3 点は**実機でしか決まらない**。データシートには PS のピン名もタイミングも書かれていない（`pdftotext` で `passive` を検索して 0 件）。

### 3-3 ツールチェーン（一次資料: GitHub のソースとスクリプト）

出典:
- https://github.com/pablomarx/rodinia（mmicko のフォーク: https://github.com/mmicko/rodinia ）
  - `bitstream/`: SPI ビットストリームの展開・解説・再パック（`agm-unpack.py` / `agm-explain.py` / `agm-pack.py`）。**対応デバイス: AG1280Q32、AG1280Q48、AG1KLPQ48、AG10KSDE176、AG16KSDE176（非圧縮のみ）**。`ag16k-wires.json.gz` もある。
  - `supra/`: Supra の配布物を解読したツールと、そのコピー。
  - `examples/`: `blinky`、`blinky-pll`、`blinky10k` ほか。**Yosys + Supra の `af`** を使う。
  - `nextpnr/`: nextpnr-generic のドライバ。AG1KLPQ48 のみ、作業中。

流れ（`examples/blinky-pll/synth.sh` と `af_run.tcl` から）:
1. `${SUPRA_DIR}/map/bin/yosys -p 'synth_agm -top Blinky -vqm Blinky.vqm' Blinky.v PLL40.v alta-bb.v` — **Supra 付属の Yosys に `synth_agm` が入っている**。出力は **`.vqm`（Quartus 風の Verilog ネットリスト）**。
2. `af -B --batch`（TCL）: `load_architect -type <DEVICE>` → `read_design_and_pack -top` → `read_sdc` → `place_pseudo -user_io -place_io -place_pll -place_gclk` → `place_and_route_design` → `bitgen normal -prg X.prg -bin X.bin`。
- `.bin` が **SPI フラッシュ用ビットストリーム**。`.prg` は別形式（用途は未確認）。
- Supra は Linux 版（`/opt/agm/Supra-2019.10.b0`）。Windows 版もあるはずだが未確認。

PLL のインスタンス化（`PLL40.v`）:
- プリミティブは **`alta_pllx`**。パラメータは `CLKIN_DIV`、`CLKFB_DIV`、`CLKOUTn_DIV`、`CLKOUTn_DEL`、`CLKOUTn_PHASE`、`FEEDBACK_MODE`、`FEEDBACK_CLOCK`。
- Supra の IP ツールで生成可能: `af -B -X "exit [catch {generate_pll {PLL40} -pll_type {PLLX} -input_freq {8} -output_freqs { {40}} -fb_mode {NO_REFERENCE}}]"`。
- 出力クロックはグローバルクロックへ **`cycloneive_clkctrl`（Cyclone IV のクロック制御プリミティブ名）**で受ける。
- 入力クロックピンは **デバイスごとに決まっている**（AG1KLPQ48 では 13 / 15 / 19 番）。AG16K のピンは別途ピンマップが要る。
- **`alta-bb.v`**（ブラックボックス定義）が必要。

NetGD への含意:
| PLAN | 内容 | 場所 |
|------|------|------|
| G1 の FPGA 本体 | 現行の `fpga/amaranth` が出す Verilog は **Cyclone II 前提のプリミティブ**（PLL、クロック制御）を持つ。Supra 流れに乗せるには **PLL を `alta_pllx` に差し替える薄いラッパ** と **`alta-bb.v`**、**`.sdc`**、**ピン定義**が要る | `fpga/amaranth/main.py`、`fpga/` に `agm/` ディレクトリを新設 |
| 検証 | **Amaranth → Verilog → `synth_agm` → `.vqm`** までは PC 上で検証できる（Supra を入手できれば）。実機に載せる前に、`examples/blinky` が通るかを AG16K で試す | |
| ライセンス | **Supra は AGM の配布物で再配布できない**。CI には載せにくい（ユーザーが各自入れる方式） | |
| 参考 | nextpnr で AG16K を通すには、チップデータベース（`ag16k-wires.json.gz` が出発点）の起こしから必要。**G1 の現実的な経路にはならない** | |

「Quartus も要る」という記述が複数の中国語ガイドにあった（AGM 用に Quartus の出力 `af_quartus.tcl` を使う古い流れ）。Supra 付属の Yosys で足りるので、**Quartus は不要**になっているようだ（二次資料: 検索要約）。

### 3-4 GDEMU 6.0 クローンや他の AG16K 基板の解析プロジェクト

- **見つからなかった。** 検索では `rodinia` 以外に AG16K の公開解析は無い。OpenGDEMU 本体は Cyclone II（v5.5 / v5.15b）が対象。
- Dreamcast-Talk の 6.0 スレ（`t=19395`）は本文が取れていない（前回から変わらず）。

---

## 4. KallistiOS `g1ata`（BBA 書き込みの設計に関係する事実）

出典（一次資料、ソースを読んだ）: https://github.com/KallistiOS/KallistiOS/blob/master/kernel/arch/dreamcast/hardware/g1ata.c と v2.1.0 / v2.2.0 タグ。

### 4-1 バージョン

| 版 | g1ata |
|----|-------|
| **2.0.0** | **無い**（`hardware/` に `sd.c`（SCIF-SPI）はあるが `g1ata.c` は無い） |
| 2.1.0 | あり。LBA48、`FLUSH_CACHE(_EXT)`、MWDMA mode 2、`data[63]` 読み取りの入った版 |
| 2.2.0 | 2.1.0 と機能は同じ。**ロックが mutex → semaphore（`sem_wait_irqsafe`）に変わった**、`irq_inside_int` 対応、リテールのときだけ終了時にマスタを選び直す |
| master | 上記 + `kos/*.h` 化。DMA 完了は `sem_count(&dma_done)` |

→ 「KOS 2.3」という名前の公式タグは無い（2.2.2 が最新タグ）。ユーザー環境の「2.3」は master の追従版と思われる。**`bba-sd-write.md` の `[KOS]` 注記はこれを足して更新する**。

### 4-2 NetGD（セカンダリ IDE）に効く動作

- **IDENTIFY の読み取り**（`g1_ata_scan`）:
  - `command_sets = word82 | word83 << 16`、**LBA48 は bit 26 = word 83 の bit 10**。
  - `capabilities = word49 | word50 << 16`、**LBA は bit 9 = word 49 の bit 9**。無いと CHS にフォールバックする（word 1/3/6）。
  - `wdma_modes = word 63`。**bit 2（0x0004）が立っているときだけ**マルチワード DMA mode 2 を有効にする（`SET FEATURES 0x03` + `0x22`）。立っていなければ **DMA を使わず PIO のみ**（`device.wdma_modes = 0`）。
  - max_lba は LBA28 なら word 60/61、LBA48 なら word 100〜103。
  - 起動時に PIO default と、可能なら WDMA2 のタイミングレジスタをプログラムする。DMA を使うときは `G1_ATA_DMA_PROTECTION` で全メモリを許可する。
- **FLUSH CACHE（`g1_ata_flush`）**: スレーブ選択 → `thd_sleep(1)` → `FLUSH_CACHE_EXT`（LBA48 のとき）または `FLUSH_CACHE` → `thd_sleep(1)` → **`g1_ata_wait_bsydrq()` で BSY|DRQ が落ちるのを待つだけ**。**エラー確認も戻り値もタイムアウトも無い**（常に 0 を返す）。シャットダウン時にも呼ばれる。
  → NetGD は FLUSH CACHE 中、**SD への書き込みが終わるまで BSY を保つ**こと。早く BSY を落とすと KOS は「書けた」と見なす。エラーは返せない（ステータスは読まれない）。
- **タイムアウト**: `g1ata.c` には `timeout` が出てこない。待ちは全て**無限ループ**（`g1_ata_wait_*`）。**NetGD 側が BSY を保ったまま固まると、DC が固まる。** SD 書き込みの失敗時は BSY を落として ERR を立てる（KOS は DMA 完了後にステータスを読む）。
- **LBA48 の使い分け**: 28 ビットに収まる範囲では LBA28 コマンドを使う（`use_lba28`）。LBA48 対応を宣言してもよいが、**NetGD が EXT コマンドを実装していないと 2^28 セクタ（128 GiB）超で壊れる**。128 GiB 以下の SD しか使わないなら LBA28 のみを宣言する（word 83 bit 10 を 0 にする）のが安全。
- **DMA**: `READ/WRITE DMA(_EXT)` は 1 コマンドで最大 65536 セクタ（LBA48）/ 256（LBA28）。PIO は 1 セクタずつ。
  → NetGD で DMA を見せるなら **word 63 bit 2 と word 53 bit 1 / word 64、SET FEATURES（0x03 / 0x22）への応答**が必要。**見せないなら PIO のみ**で動く。
- **WRITE**: PIO の `WRITE SECTORS` は 1 セクタごとにセクタ数分のデータを 16 ビット × 256 回書き込む。DMA の書き込み前に **D キャッシュをフラッシュする**（`dcache_flush_range`）— ホームブリュー側の話で NetGD は関係なし。

### 4-3 BBA 上の TCP/IP 速度（未取得）

- **実測値は見つからなかった**。lwIP と KOS 標準スタックの比較も見つからない。
- KOS 側の手掛かり（一次資料）: `examples/dreamcast/network/` に **`speedtest`**、**`lftpd`（FTP サーバ）**、`httpd`、`basic`、`ping`、`dns-client`、`ntp` がある（https://github.com/KallistiOS/KallistiOS/tree/master/examples/dreamcast/network ）。**`speedtest` は `/download-test?size=` で DC から HTTP ダウンロード速度を測る例**。実機で動かせば数字が出る。
- **`lftpd` は NetGD の G2 に流用できる**: PC から FTP でファイルを送る土台。KOS 付属なので新規に TCP サーバを書かなくてよい。
- `addons/` には `libkosfat`（FAT 実装）、`libkosext2fs` がある。**FAT32 の書き込みは `libkosfat`**（前回 `bba-sd-write.md` の前提と一致）。
- 参考値として、dcload-serial が 115200 bps で約 11 KB/s。BBA が 10 Mbps でもその数十倍にはなるはずだが、**数字は推測**。

---

## 5. CDI（音声 + データ）とセッション 2（G3 の既知バグ）

### 5-1 Flycast の扱い（一次資料）

出典: https://github.com/flyinghead/flycast/blob/master/core/imgread/cdi.cpp 、`common.h`（`Disc::GetSessionInfo`、`FillGDSession`）

- CDI は **ファイルの記述子からセッションごとに最初のトラックを拾う**。`Session.StartFAD = track.pregap_length + track.start_lba`、`FirstTrack = トラック番号`。
- `GetSessionInfo(to, session)`:
  - `session == 0`: `to[2]` = セッション数、`to[3..5]` = **ディスク全体の EndFAD**（`EndFAD = start_lba + total_length - 1`）。
  - `session N`: `to[2]` = そのセッションの最初のトラック番号、`to[3..5]` = そのセッションの StartFAD。範囲外は全て 0。
- GDI は別経路 `FillGDSession()`: **セッション 1 = トラック 1、セッション 2 = トラック 3** と決め打ち。リードアウトは 549300。
- → **CDI ではセッション境界を固定値で決めない**。NetGD の `discimg::atapi` の `boot_area_lba`（3 番以降のトラックが無いと 45000 を仮定）は GDI 向けの仮定で、CDI には合わない。**CDI の場合、`cdi.rs` が持っているセッション情報から「最後のセッションの最初のトラックの開始 LBA」を `boot_area_lba` として使う**のが筋。

### 5-2 実際の LBA（二次資料、食い違いあり）

- https://sites.google.com/site/hackerb9/cdi : ECH.CDI の例で **セッション 1 = 音声 1 トラック（LBA 0）、セッション 2 = データ（Mode2/2336、LBA 11700）、リードアウト 330827**。焼くときは**1 つ目を DAO**（TAO だとラン・アウト 2 ブロックでデータトラックの位置がずれ、ISO9660 内の絶対位置が合わなくなる）。
- 別のガイド（検索要約）: 音声 + データの CDI は「第 2 セッションを LBA **11702** から」とし、`1ST_READ.BIN` を同じ LBA に binhack するという記述がある。**11700 と 11702 の差の理由は確認できていない**（推測: 資料が数える基準が違う、または ISO9660 のボリューム記述子の位置とセッション先頭の差）。
- 自作の音声 + データ CDI ではデータセッションが 11700〜11702 付近に来る例が複数ある（二次資料）。NetGD では固定せず CDI の記述子から読むので、この差は実装には影響しない。
- BIOS / IP.BIN の期待: **セッション 2 の開始 FAD を `REQ_SES`（SEGA 0x13、セッション 2）で受け取り、そこから IP.BIN を読む**。ブート ROM はデータトラックの IP.BIN の `Sega Segakatana` 署名を確認する。NetGD で見るべきは `req_ses_reply` の `session 2` の `fad` と、TOC の最初のデータトラックの FAD が一致すること。

### 5-3 NetGD への影響

| PLAN | 内容 | 場所 |
|------|------|------|
| G3-2 の既知バグ（2 トラック CDI） | `boot_area_lba` の決め方を、**CDI は最後のセッションの先頭トラックの `start_lba`、GDI は 45000（現状）** に分ける | `common/discimg/src/atapi.rs`（`build_toc`、`req_ses_reply`）、呼び出し側の `ide.rs` |
| 追加の検証 | **実際の 2 トラック CDI**（音声 + データ）で、(a) `REQ_SES 1`、`REQ_SES 2` の応答、(b) TOC の最初のデータトラックの FAD が 150 を足した値で合っているか、(c) Flycast が返す値との差を確認する。**Flycast は `start_lba + pregap` を FAD として使い、NetGD は `start_lba + 150` を使う**ので、`pregap_length` と 150 の関係を実機イメージで確認する必要がある（推測: CDI の `start_lba` が 150 を含むかどうかで差が出る） |
| テスト | 特性テストを足す: 2 トラック CDI の記述子（音声 + データ、各 1 セッション）を入力に、`REQ_SES 0/1/2` と TOC を固定 |

---

## 6. G1 バスのタイミング・ビット反転（未取得）

- **ODE 系プロジェクトの公開資料でビット反転・DMA タイミングを扱ったものは見つからなかった**（GDEMU 公式 / クローン、MODE、USB-GDROM などを検索）。
- 得られた周辺情報（二次資料、質の低い順に参考扱い）:
  - 本家は 5.20.0 で **`read_limit`（速度制限）** と SoA / RE:CV の自動タイミングを入れた（公式のファーム履歴）。**ゲームごとのタイミング依存が既知**という根拠になる。
  - GDEMU のフリーズ原因として SD の質、電源、**本体のファン無しでの発熱**（内部クロックのリセット）が挙がっている（フォーラム伝聞）。
  - MAME の `dccons.cpp` に「G1 ATA レジスタ領域（5F70XX）に触れない」旨のコメントがあるが、実機のタイミングの話ではない。
- KOS の `g1ata.c` の**タイミングレジスタ値**（`G1_ACCESS_PIO_DEFAULT`、`G1_ACCESS_WDMA_MODE2 = 0x00001001`）は一次資料。NetGD が DMA を見せるときの目安になる。
- 上流 README の「IDE バスのビット反転は再テストしていない」は**そのまま未解決**。一次情報が無いので、**自前でロジアナ測定**（IDE の DIOR / DIOW のセットアップ・ホールド、DD 線の 3.3 V レベルと立ち上がり）をするのが唯一の道。`docs/PLAN.md` G1-4 の実機項目に「PIO 読み出しのビット誤りを全セクタ比較で数える」を足すことを勧める（推測）。

---

## 未解決の質問

1. 6.0 基板で FPGA をコンフィグするのは **誰か**（内蔵 SPI フラッシュから自動か、ATSAM3U2E の PS か）。基板の配線の実測が要る。
2. AG16K の PS 用ピン名・タイミング・ビットストリームのサイズ（非公開）。`rodinia/bitstream` の `.bin` 形式と PS 用の形式が同じか。
3. AG16KDDF256 と AG16KDDF256M の違い。内蔵 ARM が 6.0 のファームで使われているか。
4. Supra の Windows 版の入手方法とライセンス。AG16K のクロック入力ピン・グローバルクロックのピン番号。
5. `0x52 0x00` の応答の各バイトの意味（バージョンの並び）。`0x52` の他のサブコマンド（0x80 台）が本家に存在するか。GDmenu（元祖）が `0x52 0x82` 以外を使うか。
6. 本家が `0x52 0x82` の後に返す UNIT ATTENTION / SENSE の正確な系列（`open_time` / `detect_time` の間に何を返すか）。ロジアナか実機で確認。
7. `image_tag` が実在するか（公式ページには無い）。
8. SD ソケットのライトプロテクト検出が基板上にあるか（SD からのファーム更新の「LOCK を要求する」仕様を再現するため）。
9. CDI の `start_lba` と pregap と FAD の関係（Flycast は `start_lba + pregap`、NetGD は `+150`）。11700 と 11702 の差の理由。
10. KOS 標準スタックと lwIP の BBA 実測スループット。`speedtest` を実機で回す必要がある。
11. G1 バスのビット反転の原因。

---

## 出典一覧

- openMenu: https://github.com/sbstnc/openmenu （`gdemu_sdk.c` / `gdemu_sdk.h` / `gdemu_control.c` / `gd_list.c` / `gd_item.def`）
- GDMENUCardManager: https://github.com/sonik-br/GDMENUCardManager
- GDEMU 公式: https://gdemu.wordpress.com/operation/gdemu-operation/ 、https://gdemu.wordpress.com/details/gdemu-details/ 、https://gdemu.wordpress.com/firmware/updating-gdemu/
- AGM AG16K PSoC データシート: https://www.agm-micro.com/upload/userfiles/files/AG16KPSoC_DS.pdf
- rodinia: https://github.com/pablomarx/rodinia 、https://github.com/mmicko/rodinia
- KallistiOS: https://github.com/KallistiOS/KallistiOS （`kernel/arch/dreamcast/hardware/g1ata.c`、v2.1.0 / v2.2.0 / master、`examples/dreamcast/network/`）
- Flycast: https://github.com/flyinghead/flycast/tree/master/core/imgread （`cdi.cpp`、`common.h`）
- CDI 構造: https://sites.google.com/site/hackerb9/cdi
