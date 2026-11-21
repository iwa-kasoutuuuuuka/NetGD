# NetGD 計画（2026-11-20 立て直し、R111 まで反映）

NetGD は [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU) のフォーク。ドリームキャストの GD-ROM ドライブを
置き換える基板（GDEMU 系）のファームウェアと FPGA コアを、オープンソースで作る。

## 目標

| # | 目標 | 判定基準 |
|---|------|----------|
| **G1** | **GDEMU 6.0 基板で正常動作**（上流 OpenGDEMU は v5.5 / v5.15b クローンのみ対象） | 6.0 基板に書き込み、GDI/CDI のゲームが起動・プレイできる |
| **G2** | **ブロードバンドアダプタ（BBA）経由で SD カードにゲームイメージを書き込める**（本家 GDEMU では不可） | PC から BBA 経由で送ったイメージが SD に保存され、そのまま起動できる |
| **G3** | **本家 GDEMU と同等の機能を持ち、より高性能** | 機能差分表の全項目が「同等以上」。読み込み時間などの計測値が本家以上 |

## いまの状態（実測）

2026-11-20 時点（LOG R1〜R88 を反映）。根拠は `docs/research/LOG.md` の「確認」の項目。
**実機での検証はまだ無い。** 下の表の「確認」は、PC 上のビルドとテストの結果だけを指す。

**R88 時点の要約**：CI（`build.yaml`・`kos.yaml`）はコミット 35285878 で全ジョブ成功（R87）。ファームの text（`DEFMT_LOG=off`）は
4E 120,284 / 2E 120,296 / v60 66,724 / 2E `resident,sd-fast` 122,112（アプリ領域 122,880 B、512 B 丸めと CRC を入れて残り数百 B）。
`netgd` と `embassy-atsam3` の rustc 警告は 0（R74。残る約 3,100 行は自動生成の PAC の「lifetime elided」）。
下の表の件数は記録した時点のもの（その後のテストの追加は各 R を参照）。

| 項目 | 状態 | LOG |
|------|------|-----|
| MCU 4E（`mcu/netgd`、`cargo build --release`） | 成功。警告は従来どおり（R53 では 13 件）。`resident`（0x82000 リンク）版も成功 | R21, R53, R60 |
| MCU 2E（`sam3u2e`） | 成功。text 118,312 B、フラッシュの空き 約 13 KB。`resident` 版は 119,296 B で残り 3,584 B | R53, R60 |
| MCU 6.0 向け `board-v60`（`sam3u2e` + FPGA ローダー） | ビルド成功。`.rbf` を置かないと起動時に `NoBitstream`（LED 5 回点滅）。`resident` 版も成功 | R57, R60 |
| 自己更新プログラム `mcu/fwupdater` | ビルド成功（888 → 2,188 B、ファームに埋め込み）。純粋ロジックのホストテスト 5 件パス | R46 |
| 常駐ブート領域 `mcu/bootcore`（対策 B の S0） | ホストテスト 133 件以上パス（本物のフォーマッタの FAT 34 個、電源断、取り決め）。`sizeprobe` 5,902 B（8 KB に収まる） | R58, R61, R67, R70, R83, R86 |
| `common/fatmap`（VFAT の長い名前・エクステント） | ホストテスト 47 件以上パス | R82, R86 |
| `common/sdfast`（先読み予測・CMD6・検証） | ホストテストパス | R72, R75 |
| FPGA ローダー `common/psload` | ホストテスト 17 件パス（偽 FPGA、旧ループとの波形比較） | R57 |
| ホストテスト `common/lzma` | 5 件パス | R1, R46 |
| ホストテスト `common/gamecfg`（GDEMU.INI を含む） | 28 件パス | R53 |
| ホストテスト `common/discimg`（GDI/CDI・ATAPI・ATA・書き込みセッション） | 89 件パス | R53 |
| ホストテスト `tools/gdprobe` | C 47,211 チェック、compare.py 17 件パス。KOS 部分は CI でだけビルド | R31, R35 |
| ホストテスト `tools/mksdcard` | 17 件パス | R28 |
| ホストテスト `tools/netgd-send` | 42 件パス | R30 |
| ホストテスト `tools/fwbackup` | 59 件パス（probe-rs はラッパー経由のみ、0.32.0 だけ許可） | R43 |
| ホストテスト `tools/fwimage` | 17 件パス（実際の 4E・2E の ELF を使用）。形式 2 と形式 3 | R46, R60 |
| ホストテスト `tools/netgd-fwupd` | 872 項目パス（偽の更新プログラムでの成功・各エラー・再開）。KOS 部分は CI でだけビルド | R52 |
| ホストテスト `tools/netgd-expect` | 10 件パス。予測記録どうしの比較で差 0 | R55 |
| ホストテスト `tools/netgd-recv` | 中核の 29 シナリオ（quick 351 / full 567 切断点）が Windows・Linux（WSL）で通過。full は Windows 55.5 秒、Linux 11.3 秒。KOS 部分は未ビルド | R62 |
| FPGA シミュ（v5.5、Cyclone II の既定） | `test.py` 30・`test_cdda.py` 3・`test_timing.py` 5・`test_top.py` 5 件パス。Verilog はコミット済みと一致 | R1, R3, R25, R39 |
| FPGA シミュ（AG16K、`NETGD_FPGA_TARGET=ag16k`） | 同じ 4 本がパス。`test_fifo.py` 10 件で旧 FIFO と 1 サイクル単位で一致 | R39 |
| AG16K 向けブロック RAM FIFO | LUT 2,821（17 %）、M9K 14 個。オープンな Yosys の推論は確認済み、AGM の `synth_agm` では未確認 | R39 |
| CI `build.yaml` | 5 ジョブ（firmware・host-tests・gdprobe-host・tools-host・fpga-sim）。firmware は 4E・`resident`（4E・v60）・2E `resident,sd-fast` をビルド。35285878 で全成功 | R35, R50, R62, R87 |
| CI `kos.yaml`（KallistiOS 2.3.0） | `netgd-recv`・`netgd-fwupd`・`gdprobe` の ELF をビルド。35285878 で成功（実行 44717854323） | R35, R62, R87 |

**実装済み**（コード上。すべて実機未検証）：
- **G2（BBA 経由の SD 書き込み）**：F1〜F5・F13（R16）、F6〜F8（書き込みセッション・MBR 保護・FNV-1a。R38）、PC 側 `netgd-send`（R30）、Dreamcast 側 `netgd-recv`（R48。R62 で指摘 11 項目の修正を反映）。マウント時間は FAT のまとめ読み＋2回読みの CRC 照合（R68）と、`f_expand` 用の空き領域の表（R80）で対策済み（実機の `ms=` は未測定）。
- **G3（本家同等）**：番号フォルダ（`01` = GDmenu）と既定マウント、`0x52`（番号指定・次／前・バージョン応答。R14）、ボタンの番号順送り（R14、R21）、`GDEMU.INI`（`reset_goto`・`open_time`・`detect_time`。R27、R53 で R45 の指摘を修正。open/detect の NOT READY の窓は既定で無効）。CDI のデータ位置の修正（`start_lba + pregap - 150`。R36）。
  GDI の引用符付き・BOM 付きの名前（R65）と最大 128 バイトの名前（R86）、ファームの VFAT の長い名前の照合（R82）。
  **D-1（トラック2本の CDI でセッション2が返らない）は修正済み**：CDI のセッション情報で REQ_SES と起動領域を決める（R71）、セッション3つ以上とプリギャップ（R78）、ファームのトラックのセッション番号（R86）。
- **G3-4 の高速化**（すべて既定ではオフ、feature `sd-fast`）：`fast-complete`・`sd-multiblock`・`sd-highspeed`（R72、R73 のレビュー、R75 で修正）。HSMCI の CRC・タイムアウトの取りこぼしは既定のビルドでも修正済み（R75）。
- **自己更新（`0xF9`）**：対策 A（CHIPID 照合、ベクタ検査、検証後のみリセット、FWS=6 など。R46）。対策 B の S0〜S2 と、常駐部とアプリの取り決め（世代 1）の両側をホスト側まで実装（R58〜R61、R79、R83〜R86。下の「自己更新」節）。転送中の電源断などで起動不能になる窓は、対策 B の SD 復旧が実機で動くまで残る。
- **基板ごとの FPGA ローダー**：`common/psload`、feature `board-v55`（既定）／`board-v60`（R57）。6.0 のローダーのピン 5 本は v5.5 の値を仮に使っており未確認。
- **未修正**（実機と比べるまで挙動を変えない）：上流の ATAPI 応答の疑い D-2〜D-5（下の節）。

**ツール**（一行ずつ）：
- `tools/gdprobe`：純正 GDEMU と NetGD に読み取り系コマンドだけを送る KOS 製プローブ。PC 側の `compare.py` で比較（R19、R22、R31）。純正での記録はまだ取っていない。
- `tools/mksdcard`：調査用 SD（`03`〜`06`）を決定的に生成（R28）。
- `tools/netgd-expect`：NetGD が純正と同じ形式で返すはずの記録を出す。純正の記録と `compare.py --only-common` で比べる（R55）。
- `tools/fwbackup`：SWD で 6.0 の純正フラッシュを読む（`check` / `dump` / `inspect`。probe-rs 0.32.0 のみ）（R33、R40、R43）。実機では未実行。
- `tools/fwimage`：自己更新用の OGFW イメージを作る。形式 2（今の基板）と形式 3（常駐部、feature `resident`）（R46、R60）。
- `tools/netgd-send`：PC 側の送信（`prepare` / `collect` / `status`）（R30）。
- `tools/netgd-recv`：Dreamcast 側の受信（KOS + FatFs、チェックポイント、FNV-1a 照合）（R48、R62）。
- `tools/netgd-fwupd`：BBA 経由でファームを更新する KOS アプリ（形式 2、`0xF9`）。コミット済み（`a85ff42a`）。実機未検証（R52）。

**ハードウェア**：
- 実機で確かめたことはまだ無い（G1〜G3 とも P6 の実機検証待ち）。
- GDEMU 6.0 は MCU が ATSAM3U2E、FPGA が AGM AG16KDDF256（型番末尾に「M」なし、外付けフラッシュなし）。MCU から FPGA をパッシブシリアルで起動する方式がほぼ確実（R6、R41、R49）。PS のタイミングは公開されていない（R49）。
- SWD には Raspberry Pi Pico（Debug Probe）を使う。ESP32 は不可（R42）。6.0 のヘッダは穴だけでピンが無いので、はんだ付けが要る（R44）。SAM3U2E の CHIPID の期待値は `0x281A0761`（R24）。
- オーナーの作業（待ち）：純正 GDEMU での gdprobe 試験（R20、R22）、導通確認（R24、R32）、SWD で CHIPID を読み、ロックが無ければ `fwbackup dump`（R29、R43）。

**次の山場は実機**：G2 のファームと `netgd-recv` の実機試験、G1 の導通確認（分岐 A の確認）と PS のタイミングの確認、純正の記録（P5.5）。上流の疑い D は純正の実測データ待ち。

**R63・R77 の割り当ての結果**（すべて完了、LOG に記録済み）：AG16K のツールの道筋（R69：この PC では合成できない、Supra が要る）、
ビットストリームの置き場所（R66：SD の `NETGD.BIT` を既定、Supra の実サイズが約 53 KB 以下なら埋め込みも可。**R99 で実サイズは LZMA 後 37.7 KB と判明**。最終判断は PS の転送時間と実ピンでの再測定の後）、
G3-4（R72・R75）、FAT の実イメージ試験（R67・R70）、GDI の名前（R65・R86）、マウント時間（R68・R80）、P3（R74）、P5 改名（R76）。

**ソフト側の残り（実機なしでできるもの）**：
- R88 で進行中：古いコメントの修正、`read_ip_header` の起動トラックの推定を起動領域の判定にそろえる、`trim` の共用（2E −0.5 KB）。
- 2E のフラッシュの削減案 2（`opt-level="z"`、約 −7.5 KB）は、オプトインのプロファイル `release-z`・`release-z-hot-s` として用意済み（R98）。既定にするかは実機で速度を比べてから（`docs/research/flash-budget-2e.md` §7）。
- NetGD 専用メニュー `tools/netgd-menu`：設計（R97）、手順 1〜8（R101）、手順 9 の UDP 探索と手順 11 の箱絵（R108）、レビュー（R104）の指摘の修正（R109）。手順 10（ファーム更新）と 12（実機の数字で決めるもの）が残る。KOS の層は CI でコンパイルを確かめるだけで、実機は未確認。
- v5.5 用の `OpenGDEMU.rbf` は、CI（`.github/workflows/fpga.yaml`、上流の公開の Quartus のイメージ）で作れる（R103）。リポジトリの rbf は R105 のもの（番号つき ack と Status）。2E の `resident,sd-fast` は `release-z` でしか入らないので、CI もそれで確かめる（R111）。
- AG16K：Supra での手順は `fpga/agm/`（R106）。6.0 の実際のピン配置が分かれば、基板用のビットストリームを作ってファームに埋め込める（埋め込んでも約 10 KB 余る）。PS で流す時間は約 1.2〜1.7 秒と見積もった（実機で測る）。
- `embedded_sdmmc` を自前の FAT に置き換える（−4〜−7 KB の見積もり、作業は大きい）。ビットストリームを埋め込むことになった場合に検討。
- それ以外の大きな課題はすべて実機・純正の記録が要る（下の P5.5・P6）。

## フェーズ

### 基盤（G1〜G3 共通）
- **P0 ビルド復旧** — 完了。
- **P1 ドキュメント** — README / AddressMapMCU / `ide.rs` 冒頭を実装に合わせた。FPGA の古い docstring も修正（R25）。完了（レビュー済み）。
- **P2 CI** — 完了（GitHub 上で稼働。R35・R50。R50 の失敗 2 件は修正済み、R62。修正後の結果は LOG に無い）。
- **P3 警告掃除** — 完了（R74）。5種類のビルドで警告 0、フラッシュの内容はバイト単位で同一。自動生成の PAC の警告は範囲外。
- **P4 ホストテスト拡充** — 完了（R9）。ディスクイメージ解析と ATAPI 応答ビルダを `common/discimg` に切り出し済み（現在 89 件。上の表）。
  - 境界：`gdi::parse`（`gdi.rs`）のテキスト解析、`cdi::parse`（`cdi.rs`）の記述子解析、
    `ide.rs` の応答ビルダ（`bcd`・`put_msf`・REQ_SES・GET_SCD・TOC）。グローバルを読まず引数で受ける形に。
  - 手順：先に現行コードの出力を固定する特性テストを書き、切り出し前後で同じ入力→同じ出力を担保する。
- **P5 改名** — 完了（2026-11-20、R76）。クレート・バイナリ・フォルダを `netgd`（`mcu/netgd`、ELF は `target/thumbv7m-none-eabi/release/netgd`）に。IDENTIFY の機種名・FPGA のファイル名など上流の名前は残す。

### G1：GDEMU 6.0 対応
> **実物の写真で判明（2026-11-20、`docs/boards/gdemu60/README.md`）：MCU は ATSAM3U2E、FPGA は AGM AG16KDDF256。**
> MCU は既存の `sam3u2e` feature で対応できる見込み。FPGA は Cyclone II ではないため、ツールチェーン・IP・
> ビットストリームの作り直しが G1 の本体になる。フラッシュが 128 KB しかないので、ビットストリームは SD から読む方式が有力。
- 調査結果（`docs/research/gdemu6-and-parity.md`）の要点：公式の GDEMU に「6.0」は無く、6.0 はクローン独自の名称と見られる。
  AG16K の型番規則上「DD」は DDR 内蔵を意味し、ARM コア内蔵の SoC 品の可能性がある（要確認）。
  パッシブシリアルでのコンフィグに対応しているとされるが、ピン・タイミング・ビットストリームのサイズは非公開。
  開発ツール Supra は Quartus 13.1 と併用、または Yosys 内蔵という報告がある。オープンなツールは見当たらない。
- web 調査の追加（`docs/research/web-findings-2026-11.md`）：AG16KDDF256 は FPGA + Cortex コア + DDR 128 Mbit + **SPI フラッシュ**を
  1 パッケージにした品（AGM データシート）。ただし型番末尾に「M」の無い品は内蔵フラッシュを持たないとされ（R49）、基板にも外付けフラッシュが無いため、MCU からの起動になる。
  オープンなツールとして `pablomarx/rodinia`（Supra を解読。付属 Yosys の `synth_agm` → `.vqm` → `af` で P&R、PLL は `alta_pllx`）がある。
- **移植設計**：`docs/design/g1-ag16k-port.md`（Fable、LOG R32）。判断の分岐 ― A：MCU が PA23/24/25・PB1/2 で FPGA をパッシブシリアル起動（最有力）／
  A'：同じ方式で別のピン／B1：FPGA が内蔵フラッシュから自分で起動し、JTAG が出ている／B2：JTAG も無い（行き止まり）。作業2（導通確認）で決まる。
  U2 の型番末尾に「M」が無いことを写真で確認（LOG R41）、AGM のマニュアルで「M」無しは内蔵フラッシュ無しと裏づけ（R49）→ **分岐 A がほぼ確実**。
  コンフィグ用ボールの対応（R49）で、設計 §2-4 の推定は DDF256 には当てはまらないと訂正済み。
  コンフィグ方式の詳細とローダーの設計は `docs/research/ag16k-config.md`（PS のタイミングは未公開、純正の起動波形で確かめたい）。
  資源は AG16K の 17〜23 %（容量は問題なし）。FIFO のブロック RAM 対応で 2,821 LUT・14 M9K（LOG R39）。
  問題は、2E のフラッシュ（埋め込みの上限 約 46〜67 KB に対し見積もり 55〜75 KB。R54 の配置見積もりでも境界線上）と、AGM の `synth_agm` でのメモリ推論。
- **G1-1 調査** — 済（上記）。6.0 基板の MCU / FPGA / 水晶 / ヘッダ配置を公開情報から洗い出し、v5.5 との差分を表にする。
- **G1-2 実物確認（ユーザー）** — 部品の写真による確認は済（R6、R41）。導通確認（SWD・UART・ERASE パッドの位置、PA23/24/25・PB1/2 が FPGA の方へ伸びているか）はユーザーの作業待ち（R24、R32）。
- **G1-3 移植** — 進行中。ボード差分を Cargo feature（`board-v55` / `board-v60`）で切り替え、FPGA ローダーは `common/psload` に切り出し済み（R57、ホストのみ）。FIFO のブロック RAM 対応は実装済み（R39）。
  AG16K のビットストリームは Supra が要る（R69。Quartus で EP4CE15F23 として合成 → Supra の Migrate、または `synth_agm` → `af`）。**R99：Supra 2026.03（Linux 版、ネットワークなし）でビルドできた**。M9K 14 個、ロジック 13 %、LZMA 後 37,727 B。手順は `docs/research/ag16k-supra-build.md`（inout の後処理 `fixio.py` が必須、PLL は `alta_pllve`）。6.0 の実際のピン配置が分かれば、基板用のビットストリームを作れる。6.0 の PS 用ピン 5 本は v5.5 の値を仮に使用（未確認）。
- **G1-4 実機起動** — 未。P5 の手順で書き込み → UART ログ → IDE バス動作確認。

### G2：BBA 経由の SD 書き込み
- **G2-1 設計** — 済：`docs/design/bba-sd-write.md`（Fable 設計、Opus がファーム側の指摘 F1・F2・F4 をコードで確認。R8）。
  方式：FAT32 を持つのは Dreamcast 側アプリ（KOS + FatFs + `g1ata` でセカンダリ IDE ディスクに直接書く）。
  転送はまず T1（dcload-ip の `/pc/` から取得。PC 側はユーザーの Dreamcast-Dev-Tools-for-BBA にプリセットを1つ足すだけ）、
  後から T2（dcload 不要の単体 TCP プロトコル）。確定は記述子の拡張子を書き換える1セクタの書き込み1回で行う。
- **G2-2 ファーム側** — **F1〜F5・F13（R16）と F6〜F8（R38）は実装済み**（実機未検証）。
  F1 書込中はボタン無効、F2 FLUSH CACHE、F3 MWDMA2 の公表、F4 LBA48 の書込の分割、F5 取り出し中のハードリセットで復帰、
  F6 書込セッション（IMAGE_CTL）、F7 MBR 保護、F8 書込データの FNV-1a 照合、F13 機能ビット。
  性能（F9〜F12）は計測後に判断。**未着手**（書込性能、マウント解除・再スキャン、FIFO/DMA の書込経路）。
- **G2-3 コンソール側アプリ** — 実装済み：`tools/netgd-recv/`（KOS + FatFs、R48。指摘 11 項目の修正は R62）。`kos.yaml` で毎回ビルド（R87）。
  マウント時の FAT の読み取りは R68（まとめ読み）と R80（`f_expand` の空き領域の表）で対策済み。
- **G2-4 PC 側送信ツール** — 実装済み：`tools/netgd-send`（T1、R30）。dc_auto 用プリセットは `integrations/dcdevtools/netgd.ps1`。
- **G2-5 実機検証** — 未。送信 → 保存 → 起動。転送速度の計測。

### G3：本家同等 + 高性能
- **G3-1 機能差分表** — 済：`docs/research/gdemu6-and-parity.md`（Sonnet 調査、R7）。
- **G3-2 不足機能の実装** — 差分表から優先度順に。上位5件：
  > 1〜4 は**実装済み**（1〜3 は R14、4 は R27 と R53。実機未検証）。5 は未。
  1. **番号フォルダ方式（`01` = GDmenu）と、設定なしでの既定マウント。** 今は設定がないと空のまま起動する
     （`gdi.rs` `load_default`）。INSTALL.md の「最初に見つけたものをマウント」という記述も実装と食い違っている（Opus 確認済み）。
  2. GDmenu / openMenu が本家に送るイメージ切替コマンドへの対応（NetGD は独自の `0xE0` のほかに `0x52` も対応）。
     **仕様が判明**（openMenu `src/openmenu/src/backend/gdemu_sdk.c`、Opus が原文を確認）：ATAPI PACKET `0x52`。
     `52 82 lo hi` = 番号 N（1〜999、0 = 既定に戻す）をマウント、`52 81 55|44` = 次／前、`52 00` = 8 バイトのバージョン応答
     （ブートローダ版と FW 版、16 ビット単位で転送）。データ転送なしで完了時に INTRQ。メニューは SD を直接読まず、
     メニューディスク内の `OPENMENU.INI` の番号＝SD のフォルダ番号として指定する。切替後は NOT READY → MEDIA CHANGE → READY を
     約 10 秒以内に返す必要がある（openMenu は 20 ms 間隔で最大 500 回再試行）。詳細は `docs/research/web-findings-2026-11.md`。
  3. ボタンでの切替を番号順送りに（今は `GAME.CFG` の `next` を優先し、無ければ番号順。`button.rs`）。
  4. `GDEMU.ini` への対応（`reset_goto`・`open_time`・`detect_time`。R27、R53 で修正。`GDEMU.CFG` の `image=` は電源投入時のイメージを決める）。
  5. SD カードの活線挿抜と、SD からのファーム更新。SD からの復旧は対策 B（`mcu/bootcore`）で準備中。実機は S3 以降（下の「自己更新」節）。
  - トラックが2本の CDI（音声 + データ）でセッション2が返らない問題（D-1）は修正済み（R71・R78・R86）。
  - GDI のファイル名をスペース入りの引用符で書いた場合への対応は済み（R65）。
- **G3-3 性能計測** — 未。起動時間・シーク・読み込みスループットを本家と比較する方法を決め、計測する（純正の記録（P5.5）と実機（P6）が要る）。
- **G3-4 最適化** — 実装済み（R72・R75、`docs/research/g3-read-path.md`。feature `sd-fast`、既定ではオフ）。実機で速度と安定を比べてから既定にするか決める。FIFO 深さ、ATAPI 応答遅延は未着手。

### P5.5：純正 GDEMU の調査（ユーザー + Opus）
- 手順：`docs/research/stock-gdemu-flow.md`（写真 → `tools/gdprobe` → Dreamcast + BBA で実行 → 記録を比較 → LOG・テストに反映）。
- 目的：上流から引き継いだ ATAPI 応答の疑い（D-1〜D-5）の判断材料、`0x52` の実際の挙動、本家の性能の基準値（G3-3）、純正でのデータの完全性。
- 配線不要。純正ファームを壊さないよう、読み取り系コマンドだけを送る。
- 状態：**未実施**（ユーザーの作業待ち）。受け入れ準備は済み（`gdprobe` は純正で動かしてよい状態、R31。比較用の予測記録は `netgd-expect`、R55）。
  KOS 部分の初回ビルドと `switch=off` の 1 回目で確かめる（R31）。

### P6：実機検証ループ（ユーザー + Opus）
- 書き込み → UART/RTT ログ取得 → 解析 → 修正。G1-4・G2-5・G3-3 はここで回す。
- 状態：**未実施**。上の「実装済み」はすべてここで初めて確かめる。
- 既知の要確認：上流で報告されていた IDE バスのビット化け（バスタイミング）。F5 の前提（書き込み中に Dreamcast が RSTn を動かさないこと、R16）。
  書き込み中の FWS=6 の必要性（正誤表 46.1.1.3、R59）。

## モデルの役割分担

| モデル | 役割 | 担当 |
|--------|------|------|
| **Opus 5.5**（メイン） | 統括・設計判断・レビュー・難所 | 計画、全成果のレビューと統合、P6 のログ解析とタイミング系デバッグ。作業の割り当ては LOG の表（R23・R34・R47・R51・R63） |
| **Fable 5.2** | 独立した設計の深掘り・レビュー | G2-1 の設計書（R8）、G1 の移植設計（R32）、対策 B の設計（R54）、`netgd-recv` のレビューとマウント時間（R62・R63）、2E のビットストリーム置き場（R63） |
| **Sonnet 5.5** | 実装・調査 | P2 CI（済）、P4 切り出しとテスト（済）、G1-1 調査（済）、G3-1 差分表（済）、G2-2〜4 の実装、G3-2 の実装、自己更新の対策 A と対策 B の S0〜S2、R63 の調査（AG16K のツール、SD 読み込み、FAT の試験、GDI の引用符） |
| **Haiku 5.5** | 機械的・定型作業 | P1 ドキュメント（済）、FPGA の docstring（R25）、CI の Python テスト（R47）、P3 警告掃除、P5 改名、この計画書の更新（R51・R63） |

進め方：ファイルが重ならない作業は並行してサブエージェントに渡し、Opus がレビューしてからコミットする。
実機が必要な工程（G1-2、P5.5、P6）はユーザーの作業を待つ。

## 上流から引き継いだ ATAPI 応答の疑い（Fable のデバッグで判明、`docs/research/LOG.md` R11）
仕様（Sega SPI v1.30）や Flycast と食い違う点。実機（純正 GDEMU または実ドライブ）と比べるまで挙動は変えない。
直すときは `common/discimg` のテストを先に更新する。
- **D-1（高、修正済み）**：トラック2本の CDI で REQ_SES(2) が ABRT していた。CDI のセッション情報で REQ_SES と起動領域を決めるよう修正（R71・R78・R86）。純正の記録で最終確認する。
- **D-2（中、未修正）**：GET_SCD format 1 を BCD/MSF で返している（仕様はバイナリ FAD）。
- **D-3（中、未修正）**：REQ_STAT のバイト1に Disc Format が入っていない。CDB のバイト2（開始アドレス）を無視している。
- **D-4（中、未修正・要実機）**：GDI の REQ_SES がセッション数1（Flycast は2）。コード内コメントは「純正 GDEMU は1を返す」としており、実機で要確認。
- **D-5（低、一部修正）**：CDI の pregap の扱いは R36 で修正（`start_lba + pregap - 150`）。GET_SCD format 0 の R〜W ビット、lead-out の CTRL、REQ_SES のバイト0、CDI の lead-out は未修正（R45 の 8 で lead-out の影響を訂正）。
- **A-1（低、未確認）**：GDI パース中のスタックが増えた（R11 で約 140 バイト、R21 でマウント経路の最深部が約 +200 バイト）。
  2E のスタックは約 7.9 KB、マウント経路で約 5.1〜5.3 KB を使う。溢れるかは実機またはスタック解析で確かめる。
- 仕様と一致した点（R11）：TOC エントリのニブル順、first/last エントリ、CDI の area 1、Standby 時の FAD 0x96、オーディオステータス値。

## 自己更新（`0xF9`）の安全性（LOG R37・R46・R54・R58〜R61）
- 失敗すると SWD でしか戻せない（電源断、型番違いのイメージ、検証前のリセット）。判断は LOG 末尾の判断の記録（2026-11-20、D1〜D7）。
- **対策 A（実装済み、R46）**：型番照合、ベクタ検査、ページ0を最後に書く、検証後のみリセット、FWS=6、ロックの確認、2E で EFC1 に触れない。
  型番違い・壊れたイメージ・化けは防げるが、**転送中の電源断などで起動不能になる窓は残る**。
- **対策 B（推奨を承認、設計 `docs/design/fw-bootloader.md`、R54）**：8 KB の常駐部（ロック領域 0）＋ SD からの復旧（カードのルートの `NETGD.OGF` を HSMCI と FAT32 で読む）。
  - S0 `mcu/bootcore`（実装済み、R58。R59 の指摘を R61 で修正）：`no_std` の純粋ロジック、FAT32 リーダー、形式 3 の解析、SD 復旧の状態遷移、電源断の総当たり。ホストテスト 60 件。
  - S1 OGFW 形式 3（実装済み、`575d5d2d`）：`tools/fwimage`・`fwupdater`・`netgd-fwupd` で共通のベクタ。今の基板は形式 2 のまま、形式 3 は feature `resident` のときだけ。
  - S2 アプリを 0x82000 にリンクする feature `resident`（実装済み、`e6f12cff`、R60）：VTOR、クロックの冪等化、CI で両配置。`resident` のビルドではアプリ内更新の武装を拒否（S5 まで。R60）。
  - **S3 以降は実機が要る（未着手）**：常駐部の実機での割り込み・クロック、SD 初期化、ロックと EWP の両立、FPGA の遅れを Dreamcast が許すか。
    S3 のロック前の条件（R59 の 7）：mksdcard・mkfs.fat・Windows のフォーマットのイメージで FAT を試す。
    実機で確かめる項目（R59）：書き込み中の FWS=6、FLOCKE・FCMDE の読み方、4E の GPNVM2、6.0 のボタンのピン、GLB の引数 0x20 の動作。
- **残る未決**：6.0 のボタンのピンと LED の表示（D6 で保留）。SWD なしで移行するイメージは作らない（D3）。
- ホスト側のツールは揃っている：`tools/fwimage`（形式 2・3）、`tools/netgd-fwupd`（BBA 経由の送信。実機未検証）。

## 将来の地雷メモ（レビューで判明）
- slot 0・5・19 の3件は FPGA とファームの両方で対処した（`docs/design/mcu-fpga-handshake.md`、スロットの説明は `docs/AddressMapMCU.md`）。
  **シミュレーションとビルドで確認しただけで、実機では未確認。新しい動作は `out/OpenGDEMUCore.v` からビットストリームを作り直してから有効**（同梱の `OpenGDEMU.rbf` は古いまま。古いビットストリームでは新しいファームも従来どおりに動く）。
  - slot 0：`ack_command` はコマンドの番号つきで ack する。取り込みより後に来たコマンドの COMMAND_PEND は消えない（R89 A-c の残り）。
  - slot 19：書き込みにビットごとのマスク（bit 12:8）。ファームは DMA モードと PIO フロー制御をまとめて1つの「転送モード」として書き（転送は DMA か PIO のどちらかなので、片方を立てるともう片方が消えるのは意図した動作）、FPGA が持つ PKT_PHASE には触らない。
  - slot 5：MCU からの書き込みは下位バイトだけ。`set_irr` は上位バイト（LBA48 の前回値）を消さない。
- `ide_device.py` 845 行付近の「Layout」コメントと、`ide.rs` の `read_fifo_depth` のコメント（256 と 1024）。R88 で修正中。
- 2E のフラッシュ：`resident,sd-fast` で、512 B の境界までの空きは 348 B（R100）。機能を足すときは、`release-z` のプロファイル（R98）か `docs/research/flash-budget-2e.md` の削減案を先に入れる。
