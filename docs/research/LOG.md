# 調査・レビュー・判断の記録（NetGD）

すべての調査、レビュー、検証、設計判断をここに時系列で残す。新しい項目は末尾に追記する（過去の項目は書き換えず、
訂正は新しい項目として「R? の訂正」と書く）。各項目には、担当モデル、問い、結論、根拠（ファイル:行 / URL / コマンド出力）、
Opus による検証の有無、関連ファイルを書く。詳細な文書がある場合はリンクし、ここには要点を書く。

信頼度の表記：**確認済み**（コード・ビルド・テスト・一次資料で確かめた）／ **一次資料** ／ **二次資料** ／ **推測**。

---

## 2026-11-20

### R1 現状把握とビルド復旧（Opus）
- **問い**：このリポジトリは何で、今どこまで動くのか。
- **結論**：OpenGDEMU は GDEMU 互換基板（SAM3U + Cyclone II）用のオープンなファームウェアと FPGA コア。
  README の Status はC版時代のままで、Rust 版ファームは GDI/CDI・CDDA・セカンダリ IDE ディスク・IMAGE_CTL・ボタン切替・
  RAM 実行のファーム更新まで実装済み（実機未検証）。
- **根拠・検証（確認済み）**：
  - rustup の stable が壊れていた（`rustc.exe` 欠落）→ `rustup component remove/add rustc` で復旧（rustc 1.100.0）。
  - ファームのビルドが `-C inline-threshold` の削除で失敗 → `mcu/opengdemu/.cargo/config.toml` から削除（コミット `89350220`）。
  - `cargo build --release` 成功（4E）。`common/lzma` 5件、`common/gamecfg` 9件のテストがパス。
  - Amaranth 0.5.8 + amaranth-yosys 0.50.0.0.post123 を `fpga/amaranth/amaranth_env`（gitignore 済み）に導入し、
    シミュ `test.py` 30 / `test_cdda.py` 3 / `test_timing.py` 5 / `test_top.py` 5 件すべてパス。
  - このPCには probe-rs・podman・docker が無い。
- **関連**：`docs/PLAN.md`

### R2 ドキュメントと実装の食い違い（Haiku 調査、Opus がレビュー）
- **問い**：README / AddressMapMCU / `ide.rs` 冒頭はコードと合っているか。
- **結論**：3つとも古かったので更新（コミット `22a1d14a`）。更新中に見つかった Rust と Python の食い違い（未修正）：
  - `OpenGDEMU.py` の docstring はセカンダリ窓を byte `0x6300_0080`〜（128 バイト）としているが、配線と Rust では byte `0x6300_0100`（+0x80 ワード、256 バイト）。
  - `ide_device.py` の `IDEDevice` docstring が古い（FIFO スロット 9〜11 がスタブ、Data 読みが 0xFFFF など）。
  - `ide.rs` の `read_fifo_depth` のコメントはセカンダリ FIFO を 256 ワードとしているが、定数は 1024（Python と一致）。
  - キャプチャ（slot 20/21）は FPGA で `capture=False` のため常にゼロ。slot 22/23 は名前付き定数なし。
  - slot 19（DMA モードと PIO フロー制御）と slot 5（`set_irr`）は1語まるごと書き込むので、他のビットを消す。
    Opus が FPGA 側（`hw/ide_device.py` の `Case(19)` / `Case(5)`）で確認：今は転送ごとに排他、`set_irr` は ATAPI 側のみなので**無害**。将来の地雷として PLAN に記録。
  - `CMD_PACKET`（0xA0）の振り分けに役割チェックがなく、セカンダリでも `handle_packet` に入りうる。
  - `main.rs` は `fpga-diag` が無効でも slot 14〜18 を毎秒読んでいる（ログ出力だけが無効）。
- **関連**：`docs/AddressMapMCU.md`

### R3 CI の追加（Sonnet）
- **結論**：`.github/workflows/build.yaml` を追加（コミット `73f66113`）。ファームと fwupdater のビルド、ホストテスト、FPGA シミュ4本、
  `main.py` で Verilog を再生成してコミット済みの `out/OpenGDEMUCore.v` と一致するかの検査。
- **根拠（確認済み）**：ローカルで再生成した Verilog はコミット済みのものと**バイト一致**（364,190 バイト）。YAML の構文も確認。
- **未検証**：GitHub 上での実行（push していない）。

### R4 HAL の警告掃除とバイナリ差分（Haiku、Opus が判断）
- **結論**：`mcu/embassy-atsam3/src` の警告をすべて解消。4通りの feature の組み合わせすべてでビルド成功（コミット `38a9c4eb`）。
- **バイナリ差分**：作業ツリー同士の比較でファームの差は 60 バイト（.text 4、.rodata 56）。panic の行番号と定数の並び順。
  クリーンなツリー同士の比較で出た 37KB の差は、ビルド場所（パス）の違いによるもので無効（HEAD 同士でも 13KB 差が出た）。
- **判断（Opus）**：変更は未使用 import の削除、未使用関数の削除と cfg による囲い込み、未読フィールドの改名だけで、言語仕様上、動作は変わらない → **採用**。
  「バイナリ完全一致」は厳しすぎる代用指標だった。
- **訂正**：エージェントが「`OpenGDEMU.rbf` が未追跡」と報告したが誤り。`git ls-files` で追跡済みを確認（CI に影響なし）。

### R5 ユーザーの BBA 開発ツールの知見（Opus が読解）
- **出典**：https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA の `KNOWHOW_ja.md`（ユーザー自身の実機検証）。
- **要点（実機確認済み）**：BBA 転送は dcload-ip 2.0.3 で約 4.3〜5.5 MB/s、HIT-0300 は約 1 MB/s。直結・固定 IP で動く。
  `/pc/` への 128 KB の一括書き込みで dcload-ip が固まり、4 KB ずつ + fflush なら安定。noexec は電源の入れ直しでしか直らない。
  ping の応答は dcload が空いている証拠にならない。KOS 2.3 では `/cd/` リダイレクトが動かない。
  GDEMU の SD は MBR パーティションありの FAT32（スーパーフロッピーは起動しない）、フォルダは `01` からの連番（`01` = GDmenu）、ルートに `GDEMU.ini`。
- **反映**：G2 設計（R8）と G3 の差分表（R7）に渡した。

### R6 GDEMU 6.0 基板の実物（ユーザーの写真、Opus が読解）
- **結論（確認済み：刻印の目視）**：MCU は **ATSAM3U2E**（v5.5 は SAM3U4E）、FPGA は **AGM AG16KDDF256**（v5.5 は Cyclone II EP2C5）。
  水晶は 12 MHz / 11.2896 MHz、74HC04D、1117-1.2 LDO、6201E（SOT-23）。ヘッダは 5 / 3 / 8 / 2 ピンで表記なし。
  **FPGA 用の外付け SPI フラッシュは両面とも無い**（裏面写真で確認）。
- **関連**：`docs/boards/gdemu60/README.md`、写真 `top.jpg` / `bottom.jpg` / `bottom-closeup.jpg`（重複の1枚はハッシュ一致を確認して削除）。

### R7 GDEMU 6.0 と本家機能の調査（Sonnet）
- **詳細**：`docs/research/gdemu6-and-parity.md`
- **要点**：
  - 公式の GDEMU に「6.0」は無い（公式はハード v5 / v5.1、ファーム最新 5.20.5）→ 6.0 はクローン独自の名称（一次資料）。
  - SAM3U2E：フラッシュ 128 KB 単一プレーン。既存の `sam3u2e` feature で HAL の書き換えは不要、`DEFMT_LOG=off` 必須。
  - AG16K は JTAG / AS / PS でコンフィグできるとされるが、PS のピン・タイミング・ビットストリームのサイズは非公開。
  - Dreamcast-Talk の「RISC-V + CPLD」説は実物と矛盾（信頼できない）。
  - 本家との差分の上位5件：番号フォルダ方式と既定マウント／GDmenu・openMenu の切替コマンド／ボタンの番号順送り／`GDEMU.ini`／活線挿抜と SD からのファーム更新。
- **Opus の検証（確認済み）**：「設定が無いと空のまま起動する」を `gdi.rs` の `load_default` で確認。INSTALL.md §9 の「最初に見つけたものをマウント」は**実装と食い違う**（未修正）。

### R8 BBA 経由の SD 書き込み設計（Fable）
- **詳細**：`docs/design/bba-sd-write.md`
- **方式**：FAT32 を持つのは Dreamcast 側アプリ（KOS + FatFs + `g1ata` でセカンダリ IDE ディスクに直接書く。`f_expand` で領域を先に確保）。
  転送は T1（dcload-ip の `/pc/` から取得、PC 側はユーザーのツールにプリセット1つ）→ 後で T2（単体 TCP プロトコル NGTP）。
  確定は記述子を `.NGP` で書き、最後にディレクトリエントリの拡張子3バイトを書き換える1セクタの書き込み1回。
  不採用：PC がファイルシステムを持つブロック中継方式（回線が切れると FAT が壊れる）、MCU がファイルシステムを持つ方式（スタック約 2 KB と1ブロック書き込みが制約）。
- **ファーム変更**：必須 F1〜F5、推奨 F6〜F8・F13、性能 F9〜F12（PLAN の G2-2 に列挙）。
- **Opus の検証（確認済み）**：
  - F4：`embassy-atsam3/src/hsmci.rs` の `write_multi_begin` が `count > 0xFFFF` を拒否する → LBA48 の 65536 セクタ書き込みは必ず失敗。
  - F2：`ide.rs` の `handle` に FLUSH CACHE（0xE7/0xEA）の分岐が無く、既定の ABRT に落ちる。
  - F1：`button.rs` の `on_press` が `next_of_mounted` で SD を読む → 書き込み中に割り込む経路がある。
- **主なリスク**：SD 書き込み速度が未計測（1 GB で 17〜35 分の見積もり）、G1 バスのビット化け、KOS `g1ata` との組み合わせが実機未検証、
  確定処理が FatFs の内部構造に依存、ASCII 以外のフォルダ名はマウント不可、99 トラック級の GDI は `MAX_EXTENTS` 96 を超える。

### R9 ディスクイメージ解析の切り出し（Sonnet、Opus が再検証）
- **結論**：`common/discimg` を新設し、GDI/CDI 解析と ATAPI 応答（TOC / REQ_SES / REQ_STAT / GET_SCD）をホストでテスト可能に（コミット `73cd4223`）。
  テスト45件（旧コードの逐語コピー `tests/oracle` との 3000 ケースのランダム比較を含む）。
- **サイズ**：4E +28 バイト、2E +212 バイト（2E の空き約 20 KB）。`.bss` 不変。
- **意図した唯一の挙動差**：REQ_STAT と GET_SCD が cdda のスナップショットを1回だけ取るようになった。
- **テストで現状の挙動を固定した疑わしい点（未修正）**：
  1. スペース入りの引用符付き GDI ファイル名に非対応。
  2. **トラック2本の CDI（音声 + データ）でセッション2が返らない**（`boot_area_lba` が 3 番以降のトラックが無いと 45000 を仮定）。
  3. ファイル名の上位バイトは UTF-8 で2バイトに数えられ、32 バイト制限に効く。
  4. 巨大な入力でのオーバーフロー（旧コードと同じく wrapping）。
  5. `build_req_ses_reply` の `_gdi` 引数は無視されたまま。
  6. TOC の `first_ctrl` が、その領域のトラックではなく表全体で最初に見つかった同番号のトラックから取られる。
- **Opus の再検証（確認済み）**：discimg 45 / lzma 5 / gamecfg 9 件パス、4E・2E 両方のビルド成功、HAL の4通りの組み合わせで src 由来の警告ゼロ、
  P4 より前のコミット（`22a1d14a`）でもビルド成功（別の作業ツリーで確認）。
- **注意（確認済み）**：`.cargo/config.toml` の `-Map=target/openmenu.map` は相対パスなので、`--target-dir` で別の出力先を指定するとリンクが失敗する。

### R10 web 調査：未解決項目（Sonnet、Opus が一部を一次資料で検証）
- **詳細**：`docs/research/web-findings-2026-11.md`
- **要点**：
  - **GDmenu / openMenu の切替コマンド**（一次資料、**Opus が原文を確認**）：openMenu `src/openmenu/src/backend/gdemu_sdk.c`。
    ATAPI PACKET `0x52`：`52 82 lo hi` = 番号 N（1〜999、0 = 既定）をマウント、`52 81 55|44` = 次／前、`52 00` = 8 バイトのバージョン。
    NetGD の `ide.rs` に `0x52` の処理は無い（grep で確認）。
    バージョン応答のバイト順の「食い違い」は、16 ビット単位の転送をリトルエンディアンで読んだ結果として説明がつく（Opus の判断）。
  - メニューは SD を直接読まず、メニューディスク内の `OPENMENU.INI` の番号 = SD のフォルダ番号。切替後は約 10 秒以内に READY が必要。
  - 本家 `GDEMU.ini` のキーと挙動、フォルダ桁数（2〜4 桁）、`disc.*` の 8.3 名、ボタンは最後の次が `01`、活線挿抜、`reset_goto`、
    `GDEMU.FW` によるファーム更新（ライトプロテクトを LOCK にして挿す）。`image_tag` は公式ページに無い。
  - **AG16KDDF256 = FPGA + Cortex（200 MHz、SRAM 128 KB）+ DDR 128 Mbit + SPI フラッシュの1パッケージ品**（AGM データシート）。
    R6 の「外付け SPI フラッシュが無い」と整合する。6.0 が内蔵フラッシュから起動するのか、MCU が PS で送るのかは配線の実測が必要。
  - オープンなツール：`pablomarx/rodinia`（Supra を解読。付属 Yosys `synth_agm` → `.vqm` → `af`、PLL は `alta_pllx`）。nextpnr は AG1K のみ。
  - KOS `g1ata` は 2.1.0 から。IDENTIFY の word 63 bit2 が無いと PIO のみ。FLUSH CACHE は BSY|DRQ が落ちるのを待つだけ（タイムアウトなし）。
    `lftpd` と `speedtest` のサンプルが G2 で流用できそう。
  - **CDI のセッション2**：Flycast は CDI 記述子の各セッション先頭トラックの `start_lba + pregap` を返す（固定値なし）。
    NetGD の 45000 固定は GDI 向けの仮定 → R9 の疑い2を裏づける。
  - ODE のビット化けや DMA タイミングの公開資料は見つからない（ロジアナでの実測が唯一の道）。
- **未解決**：6.0 で FPGA をコンフィグする主体と PS の配線、AG16K の PS ピンとビットストリームのサイズ、Supra の入手とライセンス、
  `0x52` の他のサブコマンドと切替後の SENSE の系列、`image_tag` の実在、SD のライトプロテクト検出の有無、
  CDI の `start_lba`・pregap・FAD の関係（11700 と 11702 の差）、BBA 上の KOS TCP 実測値、G1 バスのビット化けの原因。

### R11 ここまでの成果のデバッグ（Fable）
- 実行中。完了後に結果と Opus の検証をここに追記する。

---

## 判断の記録（Decisions）

| 日付 | 判断 | 理由 | 決めた人 |
|------|------|------|----------|
| 2026-11-20 | フォーク名は **NetGD** | BBA 経由の書き込みが一番の売りで、名前から伝わる | ユーザー |
| 2026-11-20 | 目標は G1（GDEMU 6.0）・G2（BBA 経由の SD 書き込み）・G3（本家同等 + 高性能） | — | ユーザー |
| 2026-11-20 | パス・クレート名・バイナリ名（`opengdemu`）の改名は後回し | CI とドキュメントが安定してから | Opus |
| 2026-11-20 | HAL 警告掃除の 60 バイト差を許容 | 動作が変わらない変更のみ（R4） | Opus |
| 2026-11-20 | G2 は「Dreamcast 側アプリが FAT32 を持つ」方式、転送は T1 → T2 | R8 | Fable 設計、Opus 承認 |
| 2026-11-20 | git の作者は `iwa-kasoutuuuuuka` と GitHub の noreply メール（このリポジトリのみ） | メールアドレスを公開しない | ユーザー |
| 2026-11-20 | 調査・レビュー・判断はすべて `docs/research/LOG.md` に記録する | ユーザーの指示 | ユーザー |
