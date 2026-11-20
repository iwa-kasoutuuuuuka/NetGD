# NetGD 計画（2026-11-20 立て直し）

NetGD は [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU) のフォーク。ドリームキャストの GD-ROM ドライブを
置き換える基板（GDEMU 系）のファームウェアと FPGA コアを、オープンソースで作る。

## 目標

| # | 目標 | 判定基準 |
|---|------|----------|
| **G1** | **GDEMU 6.0 基板で正常動作**（上流 OpenGDEMU は v5.5 / v5.15b クローンのみ対象） | 6.0 基板に書き込み、GDI/CDI のゲームが起動・プレイできる |
| **G2** | **ブロードバンドアダプタ（BBA）経由で SD カードにゲームイメージを書き込める**（本家 GDEMU では不可） | PC から BBA 経由で送ったイメージが SD に保存され、そのまま起動できる |
| **G3** | **本家 GDEMU と同等の機能を持ち、より高性能** | 機能差分表の全項目が「同等以上」。読み込み時間などの計測値が本家以上 |

## いまの状態（実測）

2026-11-20 時点。根拠は `docs/research/LOG.md`（R1〜R50）の「確認」の項目。実機での検証はまだ無い。

| 項目 | 状態 | LOG |
|------|------|-----|
| MCU 4E（`mcu/opengdemu`、`cargo build --release`） | 成功。警告は従来どおり 13 件 | R21, R46 |
| MCU 2E（`sam3u2e`） | 成功。text 118,304 B、フラッシュの空き 約 13 KB | R46 |
| 自己更新プログラム `mcu/fwupdater` | ビルド成功（888 → 2,188 B、ファームに埋め込み）。純粋ロジックのホストテスト 5 件パス | R46 |
| ホストテスト `common/lzma` | 5 件パス | R1, R46 |
| ホストテスト `common/gamecfg`（GDEMU.INI を含む） | 21 件パス | R27 |
| ホストテスト `common/discimg`（GDI/CDI・ATAPI・ATA・書き込みセッション） | 87 件パス | R38, R46 |
| ホストテスト `tools/gdprobe` | C 47,211 チェック、compare.py 17 件パス。KOS 部分は CI でだけビルド | R31, R35 |
| ホストテスト `tools/mksdcard` | 17 件パス | R28 |
| ホストテスト `tools/netgd-send` | 42 件パス | R30 |
| ホストテスト `tools/fwbackup` | 59 件パス（probe-rs はラッパー経由のみ） | R43 |
| ホストテスト `tools/fwimage` | 17 件パス（実際の 4E・2E の ELF を使用） | R46 |
| ホストテスト `tools/netgd-recv` | 14 件パス（Windows）。Linux でのビルド不具合を修正中 | R48, R50 |
| FPGA シミュ（v5.5、Cyclone II の既定） | `test.py` 30・`test_cdda.py` 3・`test_timing.py` 5・`test_top.py` 5 件パス。Verilog はコミット済みと一致 | R1, R3, R25 |
| FPGA シミュ（AG16K、`NETGD_FPGA_TARGET=ag16k`） | 同じ 4 本がパス。`test_fifo.py` 10 件で旧 FIFO と 1 サイクル単位で一致 | R39 |
| AG16K 向けブロック RAM FIFO | LUT 2,821（17 %）、M9K 14 個。オープンな Yosys の推論は確認済み、AGM の `synth_agm` では未確認 | R39 |
| CI `build.yaml` | 5 ジョブ（firmware・host-tests・gdprobe-host・tools-host・fpga-sim）。R50 の実行で、ファーム・FPGA・gdprobe・fwbackup・fwimage・mksdcard は成功。netgd-send は修正済み（`5f4e02d8`）、netgd-recv の Linux 対応は修正中 | R35, R39, R50 |
| CI `kos.yaml`（KallistiOS 2.3.0） | gdprobe の ELF は初回でビルド成功。netgd-recv は Makefile の引用符と書式指定の不一致で失敗（run 44684499681）→ 修正中 | R35, R50 |

**実装済み**（コード上。すべて実機未検証）：
- **G2（BBA 経由の SD 書き込み）**：F1〜F5・F13（R16）、F6〜F8（書き込みセッション・MBR 保護・FNV-1a。R38）、PC 側 `netgd-send`（R30）、Dreamcast 側 `netgd-recv`（R48）。
- **G3（本家同等）**：番号フォルダ（`01` = GDmenu）と既定マウント、`0x52`（番号指定・次／前・バージョン応答。R14）、ボタンの番号順送り、`GDEMU.INI`（`reset_goto`・`open_time`・`detect_time`。R27）。CDI のデータ位置の修正（`start_lba + pregap - 150`。R36）。
- **自己更新（`0xF9`）の対策 A**：CHIPID 照合、ベクタ検査、検証後のみリセット、FWS=6 など（R46）。転送中の電源断で起動不能になる窓は残るので、対策 B が要る。
- **未変更**：上流の ATAPI 応答の疑い D-1〜D-5 は、純正 GDEMU の実測待ち（R11、R36）。R45 の指摘の一部（`0xEE` の拒否など）はコードに入っているが、修正の記録は LOG に無い。

**ツール**（一行ずつ）：
- `tools/gdprobe`：純正 GDEMU と NetGD に読み取り系コマンドだけを送る KOS 製プローブ。PC 側の `compare.py` で比較（R19、R22、R31）。
- `tools/mksdcard`：調査用 SD（`03`〜`06`）を決定的に生成（R28）。
- `tools/fwbackup`：SWD で 6.0 の純正フラッシュを読む（`check` / `dump` / `inspect`。probe-rs 0.32.0 のみ）（R33、R43）。
- `tools/fwimage`：自己更新用の OGFW v2 イメージを作る（R46）。
- `tools/netgd-send`：PC 側の送信（`prepare` / `collect` / `status`）（R30）。
- `tools/netgd-recv`：Dreamcast 側の受信（KOS + FatFs、チェックポイント、FNV-1a 照合）（R48）。
- `tools/netgd-fwupd`：BBA 経由でファームを更新する KOS アプリ。作業中（未コミット、LOG に記録なし）（R47）。

**ハードウェア**：
- 実機で確かめたことはまだ無い（G1〜G3 とも P6 の実機検証待ち）。
- GDEMU 6.0 は MCU が ATSAM3U2E、FPGA が AGM AG16KDDF256（型番末尾に「M」なし、外付けフラッシュなし）。MCU から FPGA をパッシブシリアルで起動する方式が最有力（R6、R41、R49）。PS のタイミングは公開されていない（R49）。
- SWD には Raspberry Pi Pico（Debug Probe）を使う。ESP32 は不可（R42）。6.0 のヘッダは穴だけでピンが無いので、はんだ付けが要る（R44）。SAM3U2E の CHIPID の期待値は `0x281A0761`（R24）。
- オーナーの作業（待ち）：純正 GDEMU での gdprobe 試験（R20、R22）、導通確認（R24、R32）、SWD で CHIPID を読み、ロックが無ければ `fwbackup dump`（R29、R43）。

**次の山場は実機**：G2 のファームと `netgd-recv` の実機試験、G1 の導通確認（分岐 A の確認）と PS のタイミングの確認。上流の疑い D は純正の実測データ待ち。

## フェーズ

### 基盤（G1〜G3 共通）
- **P0 ビルド復旧** — 完了。
- **P1 ドキュメント** — README / AddressMapMCU / `ide.rs` 冒頭を実装に合わせた。完了（レビュー済み）。
- **P2 CI** — 完了（GitHub 上での初回実行待ち）。
- **P3 警告掃除** — `embassy-atsam3` の未使用 import 等。ファームのバイナリ不変が条件。
- **P4 ホストテスト拡充** — ディスクイメージ解析と ATAPI 応答ビルダを `common/discimg` に切り出す。
  - 境界：`gdi::parse`（`gdi.rs`）のテキスト解析、`cdi::parse`（`cdi.rs`）の記述子解析、
    `ide.rs` の応答ビルダ（`bcd`・`put_msf`・REQ_SES・GET_SCD・TOC）。グローバルを読まず引数で受ける形に。
  - 手順：先に現行コードの出力を固定する特性テストを書き、切り出し前後で同じ入力→同じ出力を担保する。
- **P5 改名** — README / INSTALL を NetGD に。クレート名・バイナリ名（`opengdemu`）の改名は、
  CI とドキュメントが安定してから別途行う。

### G1：GDEMU 6.0 対応
> **実物の写真で判明（2026-11-20、`docs/boards/gdemu60/README.md`）：MCU は ATSAM3U2E、FPGA は AGM AG16KDDF256。**
> MCU は既存の `sam3u2e` feature で対応できる見込み。FPGA は Cyclone II ではないため、ツールチェーン・IP・
> ビットストリームの作り直しが G1 の本体になる。フラッシュが 128 KB しかないので、ビットストリームは SD から読む方式が有力。
- 調査結果（`docs/research/gdemu6-and-parity.md`）の要点：公式の GDEMU に「6.0」は無く、6.0 はクローン独自の名称と見られる。
  AG16K の型番規則上「DD」は DDR 内蔵を意味し、ARM コア内蔵の SoC 品の可能性がある（要確認）。
  パッシブシリアルでのコンフィグには対応しているとされるが、ピン・タイミング・ビットストリームのサイズは非公開。
  開発ツール Supra は Quartus 13.1 と併用、または Yosys 内蔵という報告がある。オープンなツールは見当たらない。
- web 調査の追加（`docs/research/web-findings-2026-11.md`）：AG16KDDF256 は FPGA + Cortex コア + DDR 128 Mbit + **SPI フラッシュ**を
  1 パッケージにした品（AGM データシート）。外付けの SPI フラッシュが無いのはこのためで、純正は内蔵フラッシュから起動している可能性がある。
  オープンなツールとして `pablomarx/rodinia`（Supra を解読。付属 Yosys の `synth_agm` → `.vqm` → `af` で P&R、PLL は `alta_pllx`）がある。
- **移植設計**：`docs/design/g1-ag16k-port.md`（Fable、LOG R32）。判断の分岐 ― A：MCU が PA23/24/25・PB1/2 で FPGA をパッシブシリアル起動（最有力）／
  A'：同じ方式で別のピン／B1：FPGA が内蔵フラッシュから自分で起動し、JTAG が出ている／B2：JTAG も無い（行き止まり）。作業2（導通確認）で決まる。
  U2 の型番末尾に「M」が無いことを写真で確認（LOG R41）、AGM のマニュアルで「M」無しは内蔵フラッシュ無しと裏づけ（R49）→ **分岐 A がほぼ確実**。
  コンフィグ方式の詳細とローダーの変更設計は `docs/research/ag16k-config.md`（PS のタイミングは未公開、純正の起動波形で確かめたい）。
  資源は AG16K の 17〜23 %（容量は問題なし）。FIFO のブロック RAM 対応で 2,821 LUT・14 M9K（LOG R39）。問題は非同期読み出しの FIFO がブロック RAM に割り当てられない恐れと、2E のフラッシュ（埋め込みの上限 約 67 KB に対し見積もり 55〜75 KB）。
- **G1-1 調査** — 済（上記）。6.0 基板の MCU / FPGA / 水晶 / ヘッダ配置を公開情報から洗い出し、v5.5 との差分を表にする。
- **G1-2 実物確認（ユーザー）** — 基板の両面写真とチップ型番。SWD・UART・ERASE パッドの位置。
- **G1-3 移植** — 差分に応じて HAL / ピン割り当て / FPGA ターゲット（Quartus プロジェクト）を分岐。
  ボード差分は Cargo feature（例：`board-v55` / `board-v60`）で切り替える。
- **G1-4 実機起動** — P5 の手順で書き込み → UART ログ → IDE バス動作確認。

### G2：BBA 経由の SD 書き込み
- **G2-1 設計** — 転送プロトコル、コンソール側アプリの構成、ファーム側インターロック
  （イメージマウント中は書込禁止、書込後の再マウント）を設計書にする → `docs/design/bba-sd-write.md`。
  → 済：`docs/design/bba-sd-write.md`（Fable 設計、Opus がファーム側の指摘 F1・F2・F4 をコードで確認）。
  方式：FAT32 を持つのは Dreamcast 側アプリ（KOS + FatFs + `g1ata` でセカンダリ IDE ディスクに直接書く）。
  転送はまず T1（dcload-ip の `/pc/` から取得。PC 側はユーザーの Dreamcast-Dev-Tools-for-BBA にプリセットを1つ足すだけ）、
  後から T2（dcload 不要の単体 TCP プロトコル）。確定は記述子の拡張子を書き換える1セクタの書き込み1回で行う。
- **G2-2 ファーム側** — **F1〜F5 と F13 は実装済み**（2026-11-20、LOG R16。実機未検証）。必須：F1 書込中はボタン無効、F2 FLUSH CACHE を成功で返す、F3 IDENTIFY で MWDMA2 を公表、
  F4 LBA48 の 65536 セクタ書込を分割、F5 取り出し中のハードリセットで復帰。推奨：F6 書込セッション用 IMAGE_CTL サブコマンド、
  F7 MBR 保護、F8 書込データの FNV-1a 照合。性能（計測後に判断）：F9〜F12。設計で必要と判明した変更（書込性能、マウント解除・再スキャン、FIFO/DMA の書込経路）。
- **G2-3 コンソール側アプリ** — 新ディレクトリ（例：`dc/`）に KallistiOS ベースで。
- **G2-4 PC 側送信ツール** — 例：`tools/netgd-send`。
- **G2-5 実機検証** — 送信 → 保存 → 起動。転送速度の計測。

### G3：本家同等 + 高性能
- **G3-1 機能差分表** — 本家 GDEMU の機能（メニュー/GDMenu 連携、GDEMU.INI 等の設定、IGR、
  CDDA、ディスク切替、対応イメージ形式…）を列挙し、NetGD の実装状況と対応付ける。
  → 済：`docs/research/gdemu6-and-parity.md`（Sonnet 調査）。
- **G3-2 不足機能の実装** — 差分表から優先度順に。上位5件：
  > 1〜3 は**実装済み**（2026-11-20、LOG R14。実機未検証）。
  1. **番号フォルダ方式（`01` = GDmenu）と、設定なしでの既定マウント。** 今は設定がないと空のまま起動する
     （`gdi.rs` `load_default`）。INSTALL.md の「最初に見つけたものをマウント」という記述も実装と食い違っている（Opus 確認済み）。
  2. GDmenu / openMenu が本家に送るイメージ切替コマンドへの対応（NetGD は独自の `0xE0` のみ）。
     **仕様が判明**（openMenu `src/openmenu/src/backend/gdemu_sdk.c`、Opus が原文を確認）：ATAPI PACKET `0x52`。
     `52 82 lo hi` = 番号 N（1〜999、0 = 既定に戻す）をマウント、`52 81 55|44` = 次／前、`52 00` = 8 バイトのバージョン応答
     （ブートローダ版と FW 版、16 ビット単位で転送）。データ転送なしで完了時に INTRQ。メニューは SD を直接読まず、
     メニューディスク内の `OPENMENU.INI` の番号＝SD のフォルダ番号として指定する。切替後は NOT READY → MEDIA CHANGE → READY を
     約 10 秒以内に返す必要がある（openMenu は 20 ms 間隔で最大 500 回再試行）。詳細は `docs/research/web-findings-2026-11.md`。
  3. ボタンでの切替を番号順送りに（今は `GAME.CFG` の `next` 依存、`button.rs`）。
  4. `GDEMU.ini` への対応（今は `GDEMU.CFG` の `image=` のみ）。
  5. SD カードの活線挿抜と、SD からのファーム更新。
  - P4 のテストで判明（テストで現状の挙動を固定済み、未修正）：**トラックが2本の CDI（音声 + データ）でセッション2が返らない**
    （`discimg::atapi` の `boot_area_lba` が 3 番以降のトラックが無いと 45000 を仮定する）。ホームブリューの CDI に多い構成なので優先度は高め。
    ほかに、GDI のファイル名をスペース入りの引用符で書いた場合に対応していない。
- **G3-3 性能計測** — 起動時間・シーク・読み込みスループットを本家と比較する方法を決め、計測する。
- **G3-4 最適化** — SD 読み込み（マルチブロック/DMA）、FIFO 深さ、ATAPI 応答遅延。

### P5.5：純正 GDEMU の調査（ユーザー + Opus）
- 手順：`docs/research/stock-gdemu-flow.md`（写真 → `tools/gdprobe` → Dreamcast + BBA で実行 → 記録を比較 → LOG・テストに反映）。
- 目的：上流から引き継いだ ATAPI 応答の疑い（D-1〜D-5）の判断材料、`0x52` の実際の挙動、本家の性能の基準値（G3-3）、純正でのデータの完全性。
- 配線不要。純正ファームを壊さないよう、読み取り系コマンドだけを送る。

### P6：実機検証ループ（ユーザー + Opus）
- 書き込み → UART/RTT ログ取得 → 解析 → 修正。G1-4・G2-5・G3-3 はここで回す。
- 既知の要確認：上流で報告されていた IDE バスのビット化け（バスタイミング）。

## モデルの役割分担

| モデル | 役割 | 担当 |
|--------|------|------|
| **Opus 5.5**（メイン） | 統括・設計判断・レビュー・難所 | 計画、P4 の境界設計、全成果のレビューと統合、P6 のログ解析とタイミング系デバッグ |
| **Fable 5.2** | 独立した設計の深掘り | G2-1 の設計書（プロトコル・コンソール側・ファーム側の分担） |
| **Sonnet 5.5** | 実装・調査 | P2 CI（済）、P4 切り出しとテスト、G1-1 調査、G3-1 差分表、G2-2〜4 の実装 |
| **Haiku 5.5** | 機械的・定型作業 | P1 ドキュメント（済）、P3 警告掃除、P5 改名 |

進め方：ファイルが重ならない作業は並行してサブエージェントに渡し、Opus がレビューしてからコミットする。
実機が必要な工程（G1-2、P6）はユーザーの作業を待つ。

## 上流から引き継いだ ATAPI 応答の疑い（Fable のデバッグで判明、`docs/research/LOG.md` R11）
仕様（Sega SPI v1.30）や Flycast と食い違う点。実機（純正 GDEMU または実ドライブ）と比べるまで挙動は変えない。
直すときは `common/discimg` のテストを先に更新する。
- **D-1（高）**：トラック2本の CDI で REQ_SES(2) が ABRT、`prefetch_ip_bin` も範囲外の LBA 45000 を読む。
  CDI パーサはセッション境界を知っているのに捨てている → CDI のセッション情報を `Gdi` に保持して使う。MIL-CD / selfboot の起動に直結。
- **D-2（中）**：GET_SCD format 1 を BCD/MSF で返している（仕様はバイナリ FAD）。
- **D-3（中）**：REQ_STAT のバイト1に Disc Format が入っていない。CDB のバイト2（開始アドレス）を無視している。
- **D-4（中）**：GDI の REQ_SES がセッション数1（Flycast は2）。コード内コメントは「純正 GDEMU は1を返す」としており、実機で要確認。
- **D-5（低）**：GET_SCD format 0 の R〜W ビット、lead-out の CTRL、REQ_SES のバイト0、CDI の lead-out・pregap の扱い。
- **A-1（低）**：GDI パース中のスタックが約 140 バイト深くなった（2E のスタックは約 7.9 KB、マウント経路で約 5.1 KB を使用）。
  2E で溢れないか、実機またはスタック使用量の解析で確かめる。

## 自己更新（`0xF9`）の安全性（LOG R37、未対策）
- 失敗すると SWD でしか戻せない（電源断、型番違いのイメージ、検証前のリセット）。
  **対策 A は実装済み**（LOG R46：型番照合、ベクタ検査、ページ0を最後に書く、検証後のみリセット、FWS=6、ロックの確認、2E で EFC1 に触れない）。
  型番違い・壊れたイメージ・化けは防げるが、**転送中の電源断などで起動不能になる窓は残る** → 当面は SWD で戻せる状態にしてから使う。
- 対策 A（すぐできる）：CHIPID の照合、書く前のベクタ検査、検証後のみリセット、書き込み中の FWS=6（エラッタ 46.1.1.3）、エラー後の FIFO・FSR の処理、2E で EFC1 に触れない。
- 対策 B（推奨、要判断）：ロックビットで守る常駐ブート領域。G1 のビットストリームの大きさ次第。
- 更新イメージを作って送るホスト側ツールが無い（イメージ形式も未定義）。

## 将来の地雷メモ（レビューで判明、未修正）
- slot 19：DMA モードと PIO フロー制御が1語で書かれ、片方の書込でもう片方が消える。今は転送ごとに排他なので無害。
- slot 5：`set_irr` がセクタカウントの上位バイトを消す。ATAPI 側でのみ使用するので無害。
- `OpenGDEMU.py` の docstring でセカンダリ窓のアドレスが実配線と食い違う（`docs/AddressMapMCU.md` 参照）。
