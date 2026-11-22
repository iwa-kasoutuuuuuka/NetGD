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

### R11 ここまでの成果のデバッグ（Fable、web の一次資料とも照合）
- **対象**：`main..netgd/foundation` の6コミット。
- **結論**：**ブランチが持ち込んだ機能的な退行は無い**。discimg への移行は旧実装とバイト一致
  （差分の読解、既存の 3000 件スイープ、追加の差分ファズ 80 万件 = CDI 40 万・GDI 40 万で不一致ゼロ）。
- **A：ブランチが導入した問題**
  - A-1（低、実測）：GDI マウント経路のスタックが約 +64〜140 バイト（`load_by_name` のフレーム 2748 → 2812 バイト、`PoolSink::push` と
    `strip_quotes` が非インライン化）。2E のスタックは約 7952 バイトで、マウント経路が約 5.1 KB を使う。溢れるかは静的には確定できない。
    IDE のホットパス（`handle_packet` など）のフレームは不変。
  - A-2（情報）：`tests/oracle/mod.rs` は逐語コピーではなく意味的に等価な写し（static → 引数、`ShortFileName` → `String<32>` など。ヘッダに明記）。
    関数単位で旧コードと突き合わせ、算術・分岐・エラー順序は同一 → スイープは有効。ただしオーバーフロー域は比較できていない（コード読解でのみ担保）。
    80 万件の差分ファズは恒久テストに加える価値がある。
  - A-3（低、推測）：CI の Python 3.12 で Verilog の再生成結果が 3.10 と同じかは未確認。
- **B：問題なしと確認**：GDI/CDI パーサのエラー写像と失敗時の部分状態、ATAPI 応答4種、static の寿命、defmt 配線、HAL の feature
  （`systick::init` の呼び出し側は元から `time-driver` 内、割り込みハンドラは不変）、LF の新規クローンでのファームビルド
  （相対 `-Map` パス・`xz`・fwupdater の入れ子ビルド含む）、インデックスは全ファイル LF、`OpenGDEMUCore.v` にパス属性なし、
  `docs/AddressMapMCU.md` のスロット 0〜31 が `ide_device.py` と一致。
- **C：ドキュメントとコードの食い違い**
  - C-1（確認済み、**修正**）：`ide.rs` 冒頭の「unit attention は REQUEST SENSE で読まれるまで報告」は誤り。実装は最初の1コマンドに1回だけ返す
    （`ide.rs` の `state.unit_attention = false`）。Opus が確認してコメントを修正。
  - C-2（**修正**）：README の「実機で未確認」は、上流のコード中の実機検証の記述（例：`ide.rs` の "Verified via dcload-serial against a stock GDEMU"）と紛らわしい
    → 「このフォークとしては未確認」に修正。
  - C-3（未修正、ブランチ以前から）：`mcu/opengdemu/Cargo.toml` の sam3u2e の説明にある `MAX_IMAGES` はソースに存在しない。
- **D：上流から引き継いだ挙動の疑い**（main でも同じバイト列。参照：Sega SPI 仕様 v1.30 https://segaretro.org/images/7/72/Cdif131e.pdf 、
  Flycast `core/hw/gdrom/gdromv3.cpp` と `core/imgread/`、KallistiOS `kernel/arch/dreamcast/hardware/cdrom.c`）
  - D-1（高、確認済み）：トラック2本の CDI（音声 @0、データ @11702）で `REQ_SES(2)` が ABRT。Flycast は `xx 00 02 00 2e 4c` を返す。
    `prefetch_ip_bin` も範囲外の 45000 を読む。R9 の疑い2・R10 の Flycast の調査と一致。
  - D-2（中、確認済み・**Opus もコードで確認**）：GET_SCD format 1 が BCD/MSF（`common/discimg/src/atapi.rs` の `bcd`・`put_msf`）。仕様と Flycast はバイナリ FAD。
  - D-3（中、確認済み）：REQ_STAT のバイト1に Disc Format（上位4ビット）が無い。CDB バイト2の開始アドレスを無視。
  - D-4（中、推測）：GDI の REQ_SES のセッション数が1（Flycast は2）。コード内コメントは「純正は1」と主張 → 実機で要確認。
  - D-5（低）：GET_SCD format 0 の R〜W ビット、lead-out の CTRL、REQ_SES のバイト0、CDI の lead-out（1 FAD 差）と pregap の扱い。
  - 仕様と一致した点：TOC エントリのニブル順、first/last エントリ、CDI の area 1、Standby 時の FAD 0x96、オーディオステータス値。
- **対応**：D と A-1 は `docs/PLAN.md` に課題として登録。実機と比べるまで挙動は変えない。

### R12 GitHub への公開（Opus）
- `upstream` = cepawiel/OpenGDEMU、`origin` = https://github.com/iwa-kasoutuuuuuka/NetGD（公開リポジトリ）に設定し、`main` を push。
- README を NetGD の入口として書き直した（日本語 + 英語の要約）。

### R13 G3：`0x52` 切替コマンドと番号フォルダの設計（Opus 設計、Sonnet 実装中）
- **openMenu の使い方（一次資料、Opus が確認）**：`src/openmenu/src/backend/gdemu_control.c`。ゲーム起動時に `gdemu_set_img_num(slot)`
  （= `52 82 lo hi`）を送り、`wait_cd_ready()` で `cdrom_reinit()` が成功するまで 20 ms × 最大 500 回待つ。コマンドのエラーは見ない。
  `gdemu_get_version`（`52 00`）は呼ばれていない（`main.c` の `gdemu_set_img_num(1)` もコメントアウト）。
- **設計**：
  1. 番号フォルダ：ルート直下の 2〜4 桁の数字だけの名前（値 1 以上）を番号 N とする。名前→番号と次・前の巡回は `common/discimg::slot`（ホストテスト）。
  2. 既定のマウント：選択中 → GDEMU.CFG → **フォルダ 01**（本家と同じ）。
  3. `0x52`：`82` = 番号でマウント（0 = 既定。番号が無ければ今のディスクに触らずエラー）、`81 55/44` = 次／前（欠番は飛ばす）、
     `00` = 8 バイトのバージョン（ブートローダ 0.0.0、FW は互換レベル `GDEMU_COMPAT_VERSION` = 5.20.0）。
     `0x52` は unit attention の割り込みと「ディスクなし」チェックの対象外（メニューのコマンドを握りつぶさないため）。
  4. ボタン：`GAME.CFG` の `next` を優先し、無ければ番号順の次（最後の次は 01）。
  5. マウント処理の重複（IMAGE_CTL と ボタン）を `mount_named` にまとめる。
  6. INSTALL.md §9 の誤り（「最初に見つけたものをマウント」）と Cargo.toml の `MAX_IMAGES` の記述を直す。
- **判断の理由**：番号が無いときにディスクを外さないのは、openMenu がエラーを見ずに READY を待つため、外すと 10 秒待った末に空のドライブで起動を試みるから。
  バージョンを 5.20.0 互換とするのは、本家の最新ファーム（5.20.5）相当の機能を前提とするメニューに合わせるため（openMenu 自体は見ていない）。

### R14 G3：`0x52` と番号フォルダの実装（Sonnet 実装、Opus がレビューして1件修正）
- **実装（Sonnet）**：
  - `common/discimg/src/slot.rs`：`number_of`（2〜4 桁の数字、値 1 以上、先頭ゼロ可）。
  - `gdi.rs`：`for_each_slot`（ルートを1回走査、拡張子のないディレクトリの 8.3 名だけ）、`folder_for_number`、`mounted_number`、`step_folder`。
    `load_default` は 選択中 → GDEMU.CFG → フォルダ 01 → 空。
  - `ide.rs`：`mount_named` に重複していたマウント処理を集約（IMAGE_CTL・ボタン・`0x52`）。`handle_gdemu_cmd`：
    `52 82 lo hi`（0 = 既定。番号が無ければ ILLEGAL REQUEST でディスクはそのまま）、`52 81 55/44`（次／前）、
    `52 00`（応答 `00 00 00 00 00 00 14 05` = ブートローダ 0.0.0、FW 5.20.0）、それ以外は ILLEGAL REQUEST。
    `0x52` は unit attention の割り込みの対象外（UA は保留のまま）。「ディスクなし」で事前に弾く処理は元々無かった（TEST UNIT READY と CD_READ が個別に確認）。
  - 名前は async フレームに置かず、同期関数 `stage_folder` で `MOUNT_NAME` の static に解決する → IDE タスクのフレームはむしろ縮小
    （2E の `handle` 128 → 96 バイト、4E の `handle_packet` 208 → 168 バイト）。
  - `button.rs`：`GAME.CFG` の `next` を優先し、無ければ番号順の次。
  - INSTALL.md §9 を番号フォルダ方式に書き直し、Cargo.toml の存在しない `MAX_IMAGES` の記述を削除。
- **Opus のレビューで見つけた問題（修正済み）**：`step_folder` が欠番ごとにルート全体を読み直していた（最大「最大番号」回）。
  `01` と `2024` があると、次へ進むだけでルートを約 2000 回走査し、ATAPI コマンドの処理中に数分止まってホストがタイムアウトする。
  → `discimg::slot::Nearest`（「今より大きい中で最小、無ければ全体の最小」を1回の走査で選ぶ。Prev は逆）に置き換え、走査は合計2回に。
  `step` と `max_number` は不要になったので削除。テストを書き直し（欠番・年号名のフォルダ・今の番号が無い場合・重複・空）。
- INSTALL.md の `GDEMU.CFG` の例（`image=SONICADV`）を、新しいフォルダ例に合わせて `image=02` に直した（Opus）。
- **検証（確認済み、Opus）**：discimg 53 / lzma 5 / gamecfg 9 件パス。4E・2E とも警告は従来の 13 件のみ。
  サイズ（text）：4E 150160 → 153352（+3192）、2E 110484 → 113200（+2716、空き約 17 KB）。`.bss` 不変。
- **未確認・既知の制限**：実機未検証（openMenu からの切替、切替後に READY になるまでの時間）。番号は 8.3 名だけで判定するので、
  LFN だけの `01 Title` のような名前は番号として扱わない。`52 82` は `0xE0` の名前の受け渡し用バッファ `MOUNT_NAME` を共用する
  （`0xE0` の名前を分割送信している途中に `0x52` が来ると上書きされるが、現実には起きないと判断）。

### R15 G2：ファーム必須修正 F1〜F5・F13 の方針（Opus、Sonnet 実装中）
- 設計書（`docs/design/bba-sd-write.md` §4.1）どおり F1〜F5 と F13 を実装する。Opus が補足した点：
  - **F5 はハードリセット（G1 の RSTn）のときだけ**メディアを戻す。SRST（Device Control）は KOS の `g1ata` 初期化で出うるうえ、同じバスの両デバイスに効く。
    SRST で戻すと、書き込みアプリが初期化した直後に GD イメージが再マウントされ、以後の書き込みが全部 ABRT になる。
    FPGA が見分けられる情報（RSTn のエッジ数 slot 16、`HW_IN_RESET`）で区別できなければ実装しない。
  - **F13** の機能ビットは実装済みのものだけを立てる（bit2 = DMA の公表）。bit0（書き込みセッション）と bit1（検査値）は 0。
  - IDENTIFY のワードと 65535 セクタ以下への分割は、`common/discimg::ata` の純粋関数にしてホストテストする。

### R16 G2：F1〜F5・F13 の実装（Sonnet 実装、Opus がレビュー）
- **実装**：
  - F1：`button.rs` の `on_press` は `!media_present()` なら何もしない（SD に触らない）。`service_swap_request` も取り出し中は要求を捨てる。
    `0x52` と IMAGE_CTL のマウントは取り出し中も許可（ホスト自身の操作のため）。
  - F2：Disk 役の `handle` に FLUSH CACHE（0xE7）・FLUSH CACHE EXT（0xEA）を追加。`CMD_NOP` と同じ完了（エラー 0、DRDY|DSC、INTRQ）。Packet 役は従来どおり ABRT。
  - F3：`stage_identify_disk` でワード 49 = 0x0300（LBA + DMA）、ワード 63 = 0x0407（MWDMA 0〜2 対応、2 を選択）。SET FEATURES 0x03 は元から受理。
  - F4：`handle_disk_write` を `discimg::ata::runs(lba, count, 32768)` で CMD25 の区間に分割。32768 セクタ以下は従来どおり1回の CMD25。
    読み出し側は `fetch_run` が小さい区間しか渡さないので同じ不具合は無い（報告どおり）。
  - F5：**RSTn の立ち下がりカウンタ（slot 16 の上位バイト）が前回から変わり、かつ取り出し中**のときだけ `load_media` でプライマリにメディアを戻す
    （`ide.rs` の `service_hard_reset`、`ide_task` のループで毎回呼ぶ）。
  - F13：`OGDM_PROTOCOL_VERSION` 2 → 3。VERSION 応答のバイト5 = 機能ビット 0x04（bit2 = DMA の公表のみ）。
  - `common/discimg::ata`（`advertise_mwdma`、`runs`）とテスト9件。
- **Opus の確認（確認済み）**：
  - F5 の区別の根拠：`fpga/amaranth/hw/ide_device.py:227` で `in_reset = ~ide_rstn | srst`（HW_RESET_SEEN / HW_IN_RESET は SRST と区別できない）。
    一方、`ide_device.py:897-910` の `diag_rstn_falls` は `ide_rstn` を2段の同期化 FF に通したものだけを数え、SRST は数えない。
  - F4 のエラー処理：区間内の失敗は `break` → その区間を `end()`（CMD12）で閉じる → 外側も抜ける。正しい。
  - テスト discimg 62 / lzma 5 / gamecfg 9 件パス。4E・2E とも警告は従来の 13 件のみ。
    サイズ（text）：4E 153352 → 153620（+268）、2E 113200 → 113124（-76）。`.bss` +24 バイト（タスクの future が大きくなった）。
    IDE タスクのホットパスのフレームは不変。ボタンタスクのフレームが +8 バイト。
- **未確認・リスク**：すべて実機未検証（KOS が DMA を使うか、`g1_ata_flush` の成功、65536 セクタの分割、ハードリセット後の再マウント）。
  **F5 の前提**：書き込みアプリの実行中に Dreamcast 側が RSTn を動かさないこと。KOS の `cdrom_init` などが RSTn を出す場合でも、
  それがアプリの「取り出し」より前なら影響しないが、取り出し後に出すと書き込み中に再マウントされる。実機の UART ログ
  （`service_hard_reset` のログ）で確認が必要（設計書 11 章の質問4と同じ）。リセット処理は IDE タスクが起きたとき（コマンドやボタン）にだけ走る。

### R17 方針相談：純正ファームの GDEMU 6.0 を Dreamcast + BBA 経由で使って得られる情報（Opus の見解）
- **前提（ユーザー）**：GDEMU 6.0 は手元にあるが PC と直接はつなげない。Dreamcast に装着し、BBA 経由で PC とはつなげる。
- **得られる情報（Opus の見解）**：Dreamcast 上の KOS プログラム（dcload-ip で起動）は、G1 の ATA レジスタを直接叩いて
  GD-ROM（= 純正ファームの GDEMU）にコマンドを送れる（openMenu の `gdemu_sdk.c` と同じやり方）。結果は `/pc/` に 4 KB ずつ書けば PC に残る。
  1. **純正の応答の「正解データ」**：TOC・REQ_SES・GET_SCD・REQ_STAT などの生バイトを GDI / 2 トラック CDI で取得 → R11 の D-1〜D-5
     （NetGD が仕様や Flycast と食い違う点）を、純正に合わせるべきかどうかの判断材料になる。discimg のテストの期待値にできる。
  2. **`0x52` の実際の挙動**：`52 00` の応答バイト、`52 82` 後の SENSE の系列と READY までの時間（R10 の未解決項目）。
  3. **本家の性能の基準値（G3-3）**：コマンドの応答時間、CD_READ のスループット、シーク時間、切替にかかる時間。
  4. **データの完全性**：既知のセクタを読み、PC 側の元データとハッシュを比べる → 純正でも G1 バスのビット化けが起きるかの基準。
  5. **本家の SD 運用（G3）**：`GDEMU.ini` の各キー、フォルダ番号の規則、活線挿抜などを実機で確認。
- **得られない情報**：基板の中身（MCU と FPGA の配線、AG16K のコンフィグ方式、ピン割り当て）。Dreamcast 側からは見えない。
  → G1 の移植には、テスターでの導通確認と、最初の書き込みに SWD プローブ（Raspberry Pi Pico など）が要る。
- **NetGD を書き込んだ後**：BBA が主なデバッグ経路になりうる。6.0 には UART ヘッダの表記が無いので、ファームのログを独自 ATAPI コマンドで
  読み出して BBA で PC に送る経路を作れる。ファーム更新も `0xF9`（セカンダリ経由）で BBA から行えるので、SWD が要るのは最初の1回だけ。
  ユーザーの Dreamcast-Dev-Tools-for-BBA の `dc_auto` で、実機の回帰テストを自動で回せる。
- **注意**：純正ファームに未知のコマンド（特に未知の `0x52` サブコマンドやベンダーコマンド）は送らない。ファーム更新や初期化を
  起こすものが混じっている可能性がある。既知の読み取り系コマンドだけを使う。
- **提案**：KOS のプローブツール（仮称 `tools/gdprobe`）を作り、同じテストを純正と NetGD の両方で走らせて結果を比較する（差分テスト）。

### R18 `0x52`・番号フォルダと G2 修正のデバッグ（Fable）
- **対象**：`e99a7cce~1..74946ae1 -- mcu common`。データを壊す確定的な欠陥は無し。検証はコード追跡・ビルド・逆アセンブル・KOS master `g1ata.c` との照合。
- **欠陥（修正を Sonnet に依頼中）**：
  1. **（確認済み、実害あり）イメージの無い番号フォルダに「次へ」で進むと、マウントに失敗してドライブが空になり、F1 によりボタンも効かなくなる。**
     `mount_named` が検証前に `MEDIA_PRESENT=false` にし、`load_by_name` は `.gdi/.cdi` が無いと `remember()` より前に戻るので `SELECTED_NAME` も古いまま。
     `52 81` でも毎回同じ空フォルダを選んで先に進めない。`52 82 n` も「番号はあるがイメージ無し」だと R13 の「ディスクを外さない」が守られない。
  2. （事実は確認済み、影響は推測）マウント時に SECNR のディスク形式（上位ニブル）を更新していない。`52 82` で GDI → CDI に切り替えた直後に状態を読むと古い形式が見える。
  3. （確認済み、発生確率は低い）取り出し中の CD_PLAY が `media_present()` を見ないので、cdda の SD 読み出しが書き込み中の CMD25 に割り込みうる。
  4. （確認済み、既存）`handle_disk_write` が `Stall::Reset` 後も完了ステータスを書く（`send_data_in` はリセット時に書かずに戻る）。
  5. （軽微）UA 保留中に `0x52` が失敗すると sense key が 6 → 5 に変わる。Dreamcast が使う REQ_ERROR は常にゼロなので実害はほぼ無い（修正しない）。
  6. （軽微）MWDMA を公表したのに IDENTIFY のワード 65/66 が 0（0x0078 が整合）。ワード 53 は 0x0007 で bit1 は立っている。
  7. **バージョンの符号化**：openMenu の注記は `00 00 14 05` を「5.14.0」としている → **版番号は BCD**。NetGD は 5.20 のつもりで `0x14` を返しており、5.14 に見える。
     ほかにコメントの誤り（VERSION の予約バイト数、「KOS の `g1_ata_init` が SRST を出す」→ KOS master の `g1ata.c` に SRST は無い。F5 の実装には影響しない）。
- **正しいと確認**：データ無しパケットの完了手順と openMenu の割り込み待ちの整合、`52 00` の2回の INTRQ、8.3 名の完全一致（`02 Title` に誤爆しない）、
  スタック（`stage_folder` → `step_folder` → `folder_for_number` は既存のマウント経路より浅い）、F1 の順序、F2 と KOS `g1_ata_flush` の整合、
  F3（KOS は `data[63] & 4` だけで WDMA2 を有効化）、F4 の区間境界の PIO/DMA と添字、F5 のカウンタの初期化と電源投入時の挙動、F13。
- **テストの穴**：`mounted_number` の trim、`for_each_slot` のフィルタ、マウント失敗時の進み方、ファーム側の `WRITE_RUN ≤ 0xFFFF`、実際の IDENTIFY ブロック全体は固定されていない。

### R19 プローブツール `tools/gdprobe` の方針（Opus 設計、Sonnet 実装中）
- **ビルド環境**：このPCには DreamSDK も KOS も無い（`C:\DreamSDK` なし、WSL の Ubuntu にもなし）。ユーザーの判断で、**別の PC の DreamSDK でビルド・実行**する。
  ユーザーの KallistiOS フォーク（https://github.com/iwa-kasoutuuuuuka/KallistiOS-VisualNovel 、**KOS 2.3.0**）を API の参照に使う。
  G1 バスの排他は `g1_ata_mutex_lock/unlock`（`cdrom.c` は `_g1_ata_sem` で直列化）。
- **設計**：GD-ROM に読み取り系コマンドだけを送り、生バイトと時間を `/pc/` に 4 KB ずつ記録する。純正 GDEMU と NetGD で同じ試験を走らせ、PC で比較（`compare.py`）。
  許可リスト方式（TEST UNIT READY、REQUEST SENSE、REQ_STAT、REQ_MODE、REQ_ERROR、GET_TOC、REQ_SES、GET_SCD、CD_READ、`52 00`）。
  `52 82 n`（イメージ切替）は設定ファイルで明示したときだけ（openMenu と同じ操作なので安全）。それ以外のベンダーコマンド、未知の `0x52` サブコマンド、
  SET_MODE、CD_OPEN、セキュリティ系（0x70/0x71）、セカンダリへの送信は**禁止**。すべての待ちにタイムアウト。
- KOS 部分はここではビルドできないので、純 C の部分（記録の整形、ハッシュ、4 KB 分割の書き出し、許可リスト）と `compare.py` をホストでテストする。

### R20 純正 GDEMU の調査フロー（Opus、ユーザーの依頼）
- **成果物**：`docs/research/stock-gdemu-flow.md`（手順書、Mermaid の流れ図つき）、`docs/research/probe-records/`（記録の置き場）、PLAN の P5.5。
- **流れ**：① 基板の写真（済）→ ② `tools/gdprobe` を別 PC の DreamSDK でビルド → ③ 試験用 SD（`01` メニュー、`02` dcload-ip、
  `03` GDI、`04` 2トラック CDI、`05` 3トラック以上の CDI、`06` CD 音声つき GDI、`07` 空フォルダ）→ ④ BBA で接続 →
  ⑤ `dc_auto` で実行 → ⑥ 記録を回収して `compare.py` で比較 → ⑦ LOG に記録し、`discimg` のテストの期待値やファームに反映。
- **設計上の気づき（Opus）**：dcload-ip 自体が SD 上のディスク（`02`）から起動するので、調べたいイメージへはプローブ自身が
  `52 82 n` で切り替える必要がある。→ プローブに「フォルダ番号の一覧を順に切り替えて調べる」機能を追加するよう Sonnet に依頼した。
  空フォルダ（`07`）への切替で、純正が R18 の欠陥1と同じ状況にどう応答するかも記録できる。
- 基板の中身（配線、FPGA の起動方式）はこの流れでは分からない。G1-2 の導通確認で別に調べる。

### R21 R18 の欠陥の修正（Sonnet 修正、Opus がレビューして1件追加修正）
- **修正（Sonnet）**：
  1. イメージの無い番号フォルダ：`gdi.rs` に `folder_has_image`（`.gdi/.cdi` の有無だけを見る。解析なし、static に触れない）。
     `load_by_name` の経路をたどる処理を `walk_to`・`locate_image` に切り出して共用（重複で約 2.4 KB 増えるのを避けた）。
     `52 82 n` はイメージが無ければ ILLEGAL REQUEST でディスクはそのまま。`52 81` とボタンは `discimg::slot::step_accepted` で
     イメージの無いフォルダを欠番と同じく飛ばす（拒否した番号から先へ進み、番号フォルダの数で打ち切る。k 個飛ばすと k+1 回の走査）。F1 は変更なし。
  2. `mount_named` と `load_media` の最後に `reassert_disc_format()`（リセット時と同じ処理）。失敗時は GD-ROM の値に戻る。
  3. 取り出し中の CD_PLAY は NOT READY（sense 2、エラー 0x20）。CD_SEEK は SD を読まないので変更なし。
  4. `handle_disk_write` は `Stall::Reset` で CMD25 を閉じたあと、完了ステータスを書かずに戻る（`send_data_in` と同じ）。
  5. IDENTIFY のワード 65/66 = 0x0078（120 ns）。
  6. `52 00` の版番号を BCD に（5.20.0 → `00 00 20 05`）。`common/discimg::gdemu::{bcd, version_word}` とテスト。
  7. コメントの修正（VERSION の機能バイト、SRST を KOS のものと書いていた箇所）。
- **Opus のレビューで追加修正**：ボタンの `GAME.CFG` の `next` が空フォルダを指していると、同じく空のドライブ＋ボタン無効になる
  → `folder_has_image` で確かめてから切り替え、イメージが無ければ警告を出して番号順の切替に進む（`button.rs`）。
- **検証（確認済み、Opus）**：discimg 68 / lzma 5 / gamecfg 9 件パス。4E・2E とも警告は従来の 13 件のみ。
  サイズ（text）：4E 153620 → 154040（+420）、2E 113124 → 113568（+444、空き約 17 KB）。
- **スタック（要注意）**：`load_by_name` の中から `walk_to`（0xfc）を呼ぶようになったため、マウント経路の最深部が約 +200 バイト
  （2E で推定 約 5.3 KB / 7.9 KB）。R11 の A-1 と合わせて、実機またはスタック解析での確認が必要。

### R22 `tools/gdprobe` の実装（Sonnet 実装、Opus がレビュー）と実機作業ガイド
- **実装（Sonnet）**：KOS 専用 `main.c`・`gdraw.c`、純 C（ホストテスト済み）`probes.c`・`whitelist.c`・`record.c`・`pcout.c`・`hash.c`（CRC-32）・`cfg.c`、
  `Makefile`・`build.ps1`（`$env:BBA_TOOLS` が必要）・`profiles/gdprobe.ps1`（dc_auto 用）・`gdprobe.cfg.example`、PC 側 `compare.py`、`README_ja.md`。
  設定：`label`、`switch`（既定 off）、`images=3,4,5,6,7`（範囲指定も可）、`return_to`、`switch_timeout_ms`（既定 15000）など。
  切替ループ：`52 82 n` → 20 ms ごとに TEST UNIT READY（失敗時は REQUEST SENSE、変化したときだけ記録）→ READY なら読み取り試験一式、ならなければ記録して次へ。
  記録は `gdprobe_<label>_<日時>.txt`。`identify=on`（0xA1）は既定 off。
- **ホストテスト（Opus が実行して確認）**：C 45061 チェック・`compare.py` 14 件パス。許可リストは禁止コマンド（0x12、0x16、0x20〜0x22、0x70、0x71、0xF9、`52 81`、00/82 以外の `52`）、
  ランダム 30 万 CDB、全オペコードを検査。偽ドライブ相手に UA・遅い再マウント・空フォルダ・応答停止（3回連続の時間切れで打ち切り）を試験。4 KB 以下＋毎回 flush も検査。
- **Opus のレビュー（確認済み）**：`gdraw.c` が書き込むレジスタは FEATURES・バイト数（LBA Mid/High）・COMMAND（0xA0、オプトインの 0xA1）・DATA（CDB）だけで、
  デバイス制御（SRST）やデバイス選択には書かない。すべての送信（切替ループ内の TUR・REQUEST SENSE を含む）が `wl_check` を通る。
- **未検証（KOS 部分は一度もビルドしていない）**：G1 の手順（openMenu の方式＋タイムアウト、バイト数上限 0x0800 を追加）、`g1_ata_mutex_lock` のリンク
  （`g1ata.c` で inline 定義。未定義なら `_g1_ata_sem` に切り替え）、**CD_READ の CDB バイト1 = 0x24 は記憶に基づく値で未確認**（設定 `cdread_flags` で変更可。web 調査で確認中）、
  返るセクタのサイズ（2048 / 2352。記録の `ssz=` で分かる）。
- **実機作業ガイド**：https://claude.ai/artifact/9gr3qLhDo54ME1XmiXuyDN （非公開の Artifact。同じものを `docs/research/user-tasks.html` に置いた）。
  作業1（プローブ）、作業2（導通確認。SAM3U の LQFP-144 のピン番号はデータシート Atmel-6430G の表 4-1 から：SWDIO 7、SWCLK 9、NRST 11、TDI 1、TDO 4、
  PA11 133、PA12 134、ERASE 137、PA23/24/25 = 103/105/106、PB1/PB2 = 55/57、PA20 101、PB18 25、GND 18/52/60/90/126、VDDIO 17/51/85/104/127）、
  作業3（SWD プローブの準備と、読み取りだけの CHIPID 確認）。チェックと記入はブラウザに保存され、「結果をコピー」で Claude に渡せる。
- ユーザーの依頼で、これらの作業に関する事前の web 調査を Sonnet に依頼中（結果は `docs/research/web-prep-user-tasks.md` と次の項目に記録する）。

### R23 並列作業の割り当て（Opus、ユーザーの指示「今できることを各モデルに役割を与えて並列で」）
- 実機なしでできる作業を、担当ファイルが重ならないように分けて同時に進める：
  | モデル | 作業 | 担当ファイル |
  |---|---|---|
  | Sonnet | 調査用 SD の生成ツール `tools/mksdcard`（`03`〜`06` の GDI/CDI を生成、照合用マニフェスト、`compare.py --manifest`） | `tools/mksdcard/`、`tools/gdprobe/compare.py`・`probes.c`、`common/discimg/examples/`、`stock-gdemu-flow.md` |
  | Sonnet | 実機作業の事前 web 調査 | `docs/research/web-prep-user-tasks.md` |
  | Fable | G1：AG16K への移植設計（判断の分岐、ツールフロー、資源見積もり、2E のフラッシュ） | `docs/design/g1-ag16k-port.md` |
  | Fable | `tools/gdprobe` の KOS 部分の静的レビュー（読むだけ） | なし |
  | Sonnet | G3：`GDEMU.ini` への対応 | `gdi.rs`・`ide.rs`・`button.rs`・`common/gamecfg`・INSTALL.md §9 |
  | Sonnet | G2-4：PC 側の送信ツール `netgd-send`（T1） | `tools/netgd-send/` |
  | Haiku | FPGA の古い docstring の修正（生成 Verilog が変わらないことを確認） | `fpga/amaranth/hw/*.py` |
- 成果は届いた順に Opus がレビューし、この LOG に記録してからコミットする。

### R24 実機作業の事前 web 調査（Sonnet、Opus が数値をデータシートで確認）
- **詳細**：`docs/research/web-prep-user-tasks.md`（項目 A〜E、ページへの反映点 20、未解決 10）。
- **要点**：
  - probe-rs の `SAM3U.yaml` に **ATSAM3U2E** がある（一次資料）→ 6.0 では `--chip ATSAM3U2E`（INSTALL.md の `ATSAM3U4E` は v5.5 用）。
  - **CHIPID**：アドレス 0x400E0740。**SAM3U2E = 0x281A0761、SAM3U4E = 0x28100961**（Opus がデータシートの抽出テキストの 20824〜20826 行・49213〜49214 行で確認）。
  - ロック時に probe-rs が何を表示するかの一次資料は無い。データシートでは ICE からのアクセスはすべて遮断される → `probe-rs info --verbose` で DP が見えるかどうかで、ロックか配線不良かを見分ける。
  - debugprobe は CMSIS-DAP v2（WinUSB）なので Windows でドライバ不要。2.2.3 以降を推奨（SWCLK の分周の修正）。
  - **ERASE**：15 kΩ のプルダウン、100 ms 未満の High は無視、220 ms を超えて High で消去（Opus がデータシート 1359 行・13935 行で確認）→ 電源を切った基板への導通確認は安全。
    隣の 136（SHDN）・138（TST。電源投入時に High だと書き込みモード）にプローブを滑らせない。SD カードも抜く。
  - ヘッダの仮説：v5.5 は H1 = SWD 7 ピン、H2 = 3 ピン（GND、PA29、PA31）、H3 = 5 ピン（3.3V、PC28〜31）、H4 = UART 3 ピン。
    調べるピンに JTAGSEL 142、PA29 46、PA31 48、PC28〜31 69〜72 を追加。まず全ヘッダから SWDIO（7）を探す。
  - **CD_READ の CDB バイト1**：Flycast（一次資料）では 0x20・0x24・0x28 が 2048 バイト、0x10 が 2352 バイト、0xE6 が 2340 バイト。
    Linux の `gdrom.c` は 0x20 → gdprobe の既定 0x24 は妥当。同じ FAD を 0x20/0x24/0x28/0x10 で読み比べる試験の追加を提案（後で検討）。
  - **KOS の落とし穴**：`fs_iso9660` の vblank ハンドラが、ディスクの交換を検知すると `cdrom_reinit` を呼ぶ → `52 82` の直後に KOS 自身が GD-ROM にコマンドを出し、
    プローブの直接アクセスとぶつかる恐れ。gdprobe を静的レビュー中の Fable に確認を依頼した。
  - 6.0 クローンの分解記事・ピン配置・ロックの報告は見つからない。Dreamcast-Talk の t=19395・14980・13021 と GBAtemp のスレッドは 403 で読めず。
    公開されている別のオープン実装は上流の OpenGDEMU と iceGDROM（iCE40）だけ。
- **反映**：実機作業ガイド（Artifact 版3、`docs/research/user-tasks.html`）に、`--chip ATSAM3U2E`、CHIPID の期待値と結果の読み方の表、
  `probe-rs info --verbose`、`--connect-under-reset` を使わないこと、導通確認の安全手順（SD を抜く、136・138 に触れない）、追加のピン、JTAGSEL の記入欄を加えた。

### R25 FPGA の古い docstring の修正（Haiku、Opus が確認）
- `fpga/amaranth/hw/OpenGDEMU.py`：MCU 窓の説明を、byte `0x6300_0000〜00FF`（プライマリ）／`0x6300_0100〜01FF`（セカンダリ）、各 128 個の 16 ビットスロットに修正
  （根拠：Quartus の qsf で MCU_ADDR[8:1] = SAM3U A[8:1]、`ide.rs` のセカンダリは +0x80 ワード、`main.rs` の `fpga.add(0x80 + 12)`）。R2 の食い違い1を解消。
- `fpga/amaranth/hw/ide_device.py`：`IDEDevice` の docstring を実装に合わせた（Data レジスタは読み出し FIFO、スロット 9〜11 は FIFO と水位、29〜31 は CDDA）。R2 の食い違い2を解消。
- **確認（Opus）**：差分は docstring のみ。シミュ 30/3/5/5 件パス（Haiku 実行）、生成 Verilog はコミット済みのものと一致（Opus が `git diff --exit-code` で確認）。
- 残り（未修正）：`ide_device.py` 845 行付近の「Layout」コメントがスロット 0〜12 しか挙げず、スロット 12 を「Debug test reads 0xC0DE」としている。`ide.rs` の `read_fifo_depth` のコメント（256 と 1024）。

### R26 `tools/gdprobe` の KOS 部分の静的レビュー（Fable、Opus が要点を確認して一部を修正）
- **最重要（確認済み、修正済み）**：`g1_ata_mutex_unlock()` が `g1_ata_select_device(G1_ATA_MASTER)` を呼び、**毎コマンド後に DEVICE_SELECT（0xA05F7098）へ 0x00 を書いていた**
  （KOS フォーク `g1ata.c:216-222`。Opus が確認）。KOS の cdrom ドライバも openMenu もこの書き込みはしない。→ `cdrom.c:57` の `_g1_ata_sem` を直接 `sem_wait`/`sem_signal` に変更（コミット `2e3ae3f8`）。
- **高（仕様で確認、修正を Sonnet に依頼中）**：`0x03`（REQUEST SENSE）は GD-ROM の SPI 仕様に**存在しない**（表 6.1 は 00h、10h〜16h、20h〜22h、30h、31h、40h のみ。Opus が `spi.txt` 1274 行などで確認）。
  センスは REQ_ERROR（0x13、10 バイト）で取る。純正に「誰も送ったことのないコマンド」を送るところだった。
- **高（推測、修正済み）**：PACKET 直後に前のコマンドの CHECK ビットを誤検出すると CDB を送らず固まる → パケット段階では ERR を見ず DRQ だけを待つ（openMenu と同じ）。
- **中（確認済み、修正済み）**：KOS の `fs_iso9660` の vblank ハンドラはディスクの交換を検知しても `cdrom_reinit` は呼ばない（R24 の懸念は当たらない）。ただし毎フレーム `cdrom_get_status`（BIOS の check drive）を呼ぶ。
  排他は `_g1_ata_sem` で効いている。→ 記録を汚さないよう `KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` で cdrom_init と ISO9660 を外した（`main.c`）。KOS は GD-ROM のコマンド割り込みをフックしていない。
- **中（確認済み、修正済み）**：IDENTIFY（0xA1）の応答は GD-ROM では 80 バイト（SPI 表 3.5）→ DRQ が立っている間だけ読む。
- **中（修正を依頼中）**：`profiles/gdprobe.ps1` が `-BbaTools` を渡さない、README が勧める「プリセットのコピー」でパスが壊れる。
- **CD_READ のバイト1（仕様で確認）**：bit5 = Data、bit3-1 = 期待データ型（010 = Mode 1、100 = Mode 2 Form 1）、bit0 = FAD。0x24 は GDI 向けに正しい。CDI は 0x28、任意型は 0x20。
  NetGD の `handle_cd_read` はバイト1を見ずに常に 2048 バイトで返す。
- 正しいと確認：コンパイル・リンクの API、Makefile と build.ps1 の慣習、G1 レジスタのアドレス、PIO データインの手順（複数 DRQ ブロックにも対応）、各 CDB のレイアウト、許可リスト、/pc/ の 4 KB 書き出し。

### R27 G3：`GDEMU.INI` への対応（Sonnet 実装、Opus がビルド・テストを確認）
- `common/gamecfg::ini`（テスト 12 件追加、計 21 件）。`reset_goto=N`：ハードリセット（RSTn のカウンタ）でイメージ N をマウント（無ければ何もしない）。
  `open_time`・`detect_time`（150〜5000 ms）：切替後にその合計の時間 NOT READY を返す（ファイルで指定したときだけ。INI が無ければ従来どおり）。
  `image_tests`・`high_speed`・`read_limit`・未知のキーは読み飛ばしてログを出す。INI はカードを読むたびに読み直す。`GDEMU.CFG` の `image=` は引き続き電源投入時のイメージを決める。
- **確認（Opus）**：gamecfg 21 / discimg 68 / lzma 5 件パス、4E・2E とも警告は従来の 13 件。2E の text 115944（空き約 14.6 KB）。コミット `9cbffeca`。
- **未確認・要検討**：根拠は公式ページの要約メモ（`web-findings-2026-11.md` 2-1）で、原文の引用ではない。切替後の UA は NOT READY の前に1回返しており、本家の順序（NOT READY → MEDIA CHANGE → READY）とは違う可能性がある。
  INI が無いときに本家の既定値（500/250 ms）を当てていない。→ 純正のプローブ記録（切替後の SENSE の並びと時間）で確かめてから合わせる。

### R28 調査用 SD の生成ツール `tools/mksdcard`（Sonnet 実装、Opus がコミット）
- `03`（GDI）・`04`（2トラック CDI、データは LBA 11702）・`05`（音声2本＋データの CDI）・`06`（音声3本の GDI）を決定的に生成。全セクタに画像番号・トラック・LBA のタグと疑似乱数。
  CDI の書き出しは mkdcdisc（https://gitlab.com/simulant/mkdcdisc 、MIT）に項目単位で合わせた。合計 125 MB（`--small` で約 17 MB）。
- 検証：`verify.py` で4つとも NetGD のパーサ（新しい `discimg/examples/inspect.rs`）で読め、マニフェストと一致。`compare.py --manifest` で、イメージファイルなしに期待 CRC を再計算できる。テスト mksdcard 17・compare 17 件。コミット `a1f9a294`。
- **NetGD 側の疑い（新規）**：mkdcdisc は音声が2本以上の CDI で、トラックの `start_lba` を pregap の先頭に置く。NetGD はそれをデータの先頭として扱うので、`05` のトラック2が 150 セクタずれる（R11 の D-5 の「CDI の pregap の扱い」と同じ根）。
- **未確認**：純正 GDEMU が生成した CDI を受け付けるか。Mode 1 の P/Q ECC は独自の参照実装でしか確かめていない（mkdcdisc の libedc とのビルド比較は権限で拒否）。

### R29 GDEMU 6.0 の純正ファームは吸い出せるか（ユーザーの質問、Opus の回答）
- **根拠（データシート、Opus が確認）**：SAM3U のセキュリティビットは GPNVM bit 0（8.2.3.5、20.4.3.6）。有効なときは ICE（JTAG/SWD）と Fast Flash Programming Interface からの
  フラッシュ・SRAM・コアレジスタ・内部周辺へのアクセスがすべて禁止される。解除は ERASE ピンを 1 にしてフラッシュ全体を消去するしかない。
- **結論**：吸い出せるかどうかは、**6.0 の純正がこのビットを立てているか**で決まり、まだ誰も確かめていない（R24：公開情報なし）。
  - 立っていない（作業3の CHIPID が `0x281A0761` で読める）→ SWD でフラッシュ 128 KB を読み出せる見込み。**何かを書き込む前にバックアップを取る**よう実機作業ガイドに追記した（Artifact 版4）。
  - 立っている（CHIPID も読めない）→ 正規の手段では吸い出せない。ERASE で消すと二度と戻らない。
  - 公式の `GDEMU.FW`（SD からの更新ファイル）は公式ハード向けで、暗号化されている可能性が高く、クローンのファームとも限らない。電圧グリッチなどでセキュリティを破る方法は、基板を壊す危険があり勧めない。
  - FPGA（AG16K）の内蔵フラッシュの中身を読み出せるかは不明。
- INSTALL.md や README の「吸い出せない」は v5.5 クローンでの上流の経験に基づく記述で、6.0 では未確認。

### R30 G2-4：PC 側の送信ツール `tools/netgd-send`（Sonnet 実装、Opus がテストを確認）
- 設計書 §7.1 のパッケージ構成。`prepare`（GDI/CDI の検査、8.3 名への変換と `disc.gdi` の書き直し、8 MiB ごとのチェックポイント CRC、`job.ngj`・`manifest.ngm`・`src.txt`）、
  `collect`（Dreamcast 側が書く `result.ngr` を検証。終了コード 0 成功／1 失敗／2 未完了で再開可）、`status`。ファイルはハードリンクで置き、複製しない。
  dc_auto 用プリセット `integrations/dcdevtools/netgd.ps1`（Dreamcast 側アプリ `netgd-recv` は未作成なので ELF は仮のパス）。
- **テスト（Opus が実行）**：42 件パス（約 30 秒、ファイル I/O が遅いため）。プリセットは PowerShell で構文だけ確認、dc_auto では未実行。
- **設計の穴（DESIGN-GAP、12 件。コード中に印あり）**：主なもの ― ファイルは分割せずチェックポイントの境界を「チャンク」とする、PC 側にだけ `manifest.ngc` を追加、
  `src.txt` のパスは `/pc/...`、既定の SD フォルダの決め方、`result.ngr` の行の形式（`NGD:RESUME`・`NGD:FILE`・`NGD:END` などは提案）、
  CDI の末尾の検査は記憶に基づく、設計の `--prepare` フラグをサブコマンドにした。→ Dreamcast 側アプリ（G2-3）を作るときに合わせて確定する。

### R31 `tools/gdprobe` の残りの修正（Sonnet 実装、Opus が確認）
- `0x03`（REQUEST SENSE）を許可リストと送信経路から削除し、センスは REQ_ERROR（0x13、10 バイト：バイト2下位 = sense key、バイト8 = ASC、バイト9 = ASCQ）で取る。
  ウォームアップは `REQ_ERROR_W<i>`、切替ループのポーリングも 0x13。**Opus が `probes.c`・`whitelist.c` に 0x03 が残っていないことを grep で確認。**
- `profiles/gdprobe.ps1`：`-BbaTools $DevkitRoot` を渡す。dc_auto の `-Preset` はフルパスを受け付けるので、コピーの案内を削除。
- README：CD_READ のバイト1（0x24 は仕様で確認、0x20／0x28 の使い分け、`switch=off` で dcload-ip の CDI を読むなら `cdread_flags=0x20`）、
  KOS の初期化フラグ、`_g1_ata_sem` を直接使う理由、バイト数レジスタ、IDENTIFY が 80 バイトであることを反映。
- IP を読む FAD の決め方：設定 `ip_fad` → REQ_SES(2) → GET_TOC の最後の有効な領域の最初のデータトラック → 45150。CDI のセッション境界は TOC に無いので推測であり、複数セッションの CDI では `ip_fad=` の指定を残す。
- **確認（Opus）**：C 47211 チェック・Python 17 件パス、`gdraw.c`・`main.c` の構文チェック OK。
- これで R26 の指摘はすべて対応済み。**純正でプローブを動かしてよい状態**（ただし KOS 部分は未ビルドのまま。初回ビルドと `switch=off` の1回目で確かめる）。

### R32 G1：AG16K への移植設計（Fable、Opus が一部を確認）
- **詳細**：`docs/design/g1-ag16k-port.md`。
- **判断の分岐**（作業2の導通確認で決まる）：A = PA23/24/25・PB1/2 が v5.5 と同じく FPGA へ（パッシブシリアル。`cyclone.rs` を AG16K 用に調整）、最有力／
  A' = 同じ方式で別のピン（`board-v60` で切り替え）／B1 = MCU からのコンフィグ線が無く、FPGA の JTAG がヘッダ（8 ピンが候補）に出ている
  → FPGA の内蔵フラッシュを先に吸い出し、JTAG で NetGD のビットストリームを一度だけ書く／B2 = JTAG も無い → 基板改造なしでは行き止まり／
  C = 判別できない → ERASE の前に、電源投入時の波形、MSEL、JTAG の IDCODE で確かめる。
  A が有力な理由：型番末尾に「M」の無い品は FPGA + DDR だけで内蔵フラッシュが無い可能性がある（二次資料）。外付けフラッシュも無いので、MCU からの起動になる。
  純正の FPGA ビットストリームを流用する案は不可（レジスタ配置が不明、A の場合は MCU のファームの中にあって ERASE で消える）。
- **資源の見積もり**（Yosys 0.71、YoWASP。venv の amaranth-yosys は機能が削られていて使えなかった）：LUT4 2,811／FF 1,041 → AG16K（16,624 LE）の 17〜23 %。
  メモリ 約 14 M9K／56（約 20 %）、PLL 1／4、I/O 66／169。**容量は問題ない**。
  **問題はメモリの推論**：5つの FIFO がすべて非同期読み出しで、オープンな Yosys は M9K に割り当てず、LUT/FF で約 7 倍に膨らむ。Quartus は自動で直すが、Supra の `synth_agm` が直すかは不明。
- `core_pll` は出力が使われておらず不要。移植が要るのは `Audio_PLL`（11.2896 MHz ×3）だけ。AG16K の PLL のプリミティブ名は未確定。
- **2E のフラッシュ**（作業中の作業ツリーで計測）：コード 64,052 + `.flash1`（LZMA の Cyclone ビットストリーム）49,632 = 113,684 / 131,072。埋め込めるビットストリームの上限は約 67 KB。
  AG16K の非圧縮ビットストリームは 552,366 バイト（rodinia のサンプル）、NetGD の設計を LZMA で圧縮すると 55〜75 KB と見積もり → 境界線上。Supra で一度ビルドすれば（基板なしで）分かる。
  パッシブシリアルのビット数は今の約 7.5 倍になり、起動時間（ドライブが応答しない時間）が延びる。
- **リスク**：BGA のピン配置（約 66 信号）が不明、ERASE の不可逆性（2枚目の基板の用意と、先に純正のプローブ記録を取ることを推奨）、Supra の入手とライセンス、FIFO のメモリ推論、フラッシュ容量、起動時間。
- **Opus が確認した食い違い**：上流の `docs/PCB.md` は FPGA の 48 MHz を PA28（PWMH0）としているが、ファームは PCK0 を **PA21** に出している（`embassy-atsam3/src/lib.rs:160-162`、
  `clocks.rs:95`「PCK0 drives the FPGA's CLK_48_MHz pin」、Quartus では FPGA の PIN_91）。→ 実機作業ガイドの導通確認に PA21（102 番）と PA28（45 番）を追加した（Artifact 版6）。
- 未レビュー：2E の単一フラッシュプレーンでの `fwupdate` / `fwupdater` の動作。

### R33 純正ファームの吸い出しツール `tools/fwbackup` の方針（Opus 設計、Sonnet 実装中）
- **ユーザーの希望**：吸い出せるならバックアップとして保存したい。
- **設計**：PC 側の Python スクリプト。probe-rs（Pico の debugprobe 経由の SWD）を使う。
  `check`（CHIPID・EXID を読み、ロックなし 2E／ロックなし別型番／ロックまたはアクセス不可／応答なし に分類）、
  `dump`（ロックなしのときだけ、フラッシュ 128 KB = 0x80000〜0x9FFFF を2回読んで一致を確認し、`.bin`・SHA-256・JSON を保存。上書きしない）、
  `inspect`（ベクタテーブルの妥当性、空領域、文字列の数など）。
- **安全**：probe-rs のサブコマンドは `list`・`info`・`read` だけを許可するラッパーを通す（`download`・`erase`・`reset`・`--connect-under-reset` などは拒否、テストで確認）。
  `probe-rs read` がコアを止めるかどうかを実装前に一次資料で確認する。
- **法的な扱い**：吸い出したファームは元のメーカーの著作物。個人のバックアップとして非公開で保管し、コミットもアップロードもしない（`.gitignore` で除外）。NetGD はその中身を使わない。
- 実装後に Fable が安全面をレビューする。

### R34 実機なしで進める作業の並列割り当て（Opus、ユーザーの指示「全部、各モデルに役割を与えて並列で」）
| # | モデル | 作業 | 担当ファイル |
|---|---|---|---|
| 1 | Sonnet | KOS のビルドを GitHub Actions で（プローブと `netgd-recv` のコンパイル確認） | `.github/workflows/kos.yaml` |
| 2 | Sonnet | G2-3：Dreamcast 側の受信アプリ `netgd-recv`（FatFs、チェックポイント、電源断のホストテスト） | `tools/netgd-recv/` |
| 3 | Fable | G1：FIFO をブロック RAM に乗る形に。**v5.5 の Verilog はバイト一致を保ち**、AG16K 向けは `--target ag16k` で別に生成 | `fpga/amaranth/` |
| 4 | Sonnet | CDI の pregap の修正（R28）。D-1（セッション2）は純正データ待ちなので触らない | `common/discimg` の CDI、ファームの `cdi.rs`・`gdi.rs` の CDI 部分 |
| 5 | Sonnet | G2 推奨 F6〜F8（書き込みセッション、MBR 保護、書き込みデータの FNV-1a）と F13 の機能ビット | `ide.rs`・`sdcard.rs`・`common/discimg` の新モジュール |
| 6 | Fable | 2E での自己更新（`0xF9`）の安全性レビュー（読むだけ） | なし |
| 7 | Haiku | クレート名 `opengdemu` → `netgd` の改名、古いコメントの修正 | **ほかの作業の後**（ほぼ全ファイルに触るため） |
- 吸い出しツール `tools/fwbackup`（Sonnet、R33）も並行して実行中。
- 判断：#3 で v5.5 の Verilog を変えないのは、この PC に Quartus が無く、コミット済みの Cyclone II のビットストリームを作り直せないため。

### R35 KOS ビルドの CI が初回で成功（Sonnet 作成、Opus が push して結果を確認）
- `.github/workflows/kos.yaml`・`tools/ci/kos-build.sh`：KallistiOS プロジェクトの CI が公開する `pcercuei/dreamcast-toolchain:13.4.0`（ダイジェスト固定）と、
  ユーザーの KOS 2.3.0 フォーク（コミット `c957c5ab` に固定）で、`tools/*/Makefile` のうち KOS のものをすべてビルド。KOS 本体はキャッシュ。
- **初回の実行（run 44679084287）が成功**：`gdprobe.elf` がコンパイラの警告なしでリンクまで通った（Opus がログで確認）。成果物 `dreamcast-elfs`（約 1 MB）。
  → **プローブの KOS 部分が初めて実際にコンパイルされた**（それまでは構文チェックだけ）。
- 注意：CI 版の ELF は PC 側の保存先が `/pc/ci` に固定 → dc-tool-ip を動かすドライブの `\ci\` に `gdprobe.cfg` を置く。実機作業ガイド（Artifact 版8）に追記。

### R36 CDI の pregap の修正（Sonnet、Opus がテストを確認）／ R28 の訂正
- **R28 の訂正**：「`05` のトラック2が 150 セクタずれる」という疑いは**誤り**。pregap が 150 のとき、データの先頭は記述子の `start_lba` そのものになり、NetGD の旧実装でも正しかった。
- **実際の不具合**：記述子の `start_lba` は pregap の先頭（mkdcdisc の `cd_track_start_lba`、Flycast `cdi.cpp` のトラック FAD = start_lba + pregap）。
  NetGD はそれをデータの先頭として扱っていたので、**pregap が 150 以外のトラック**（DiscJuggler 製の CDI で後ろのトラックが pregap 0 など）を `150 - pregap` セクタ遅れて報告していた。
  ファイル上の先頭データの位置は元から正しかった。
- **修正**：`common/discimg/src/cdi.rs` でデータの LBA = `start_lba + pregap - 150`（飽和演算）。ファームは `discimg` の結果をそのまま使うので自動的に追従。
  mksdcard のイメージ（全トラック pregap 150）は変化なし（`verify.py` OK）。変わった出力はテストの pregap 0 のトラックだけ（3150 → 3000）。
  オラクルの比較は、この規則で写した `start_lba` 以外を引き続き完全一致で比較。テスト 3 件追加、discimg 87 件パス（Opus が実行）。コミット済み。
- 触っていない：REQ_SES のセッション2（D-1）、lead-out の式（D-5。純正データ待ち）。mkdcdisc は 302 セクタ未満の音声トラックをファイル上でだけ詰めるので、後ろのトラックのオフセットがずれうる（mksdcard は常に 302 以上なので影響なし）。

### R37 2E での自己更新（`0xF9`）の安全性レビュー（Fable、読むだけ）
- **結論**：書き込みの仕組みは 2E でも正しく動く見込み（SRAM1 は 2E にも 0x20080000 から 16 KB、EEFC0、EWP コマンド、更新プログラムはフラッシュ上のコードやデータに触れない ― 逆アセンブルで確認）。
  **ただし復旧経路が無く、失敗すると SWD が必要**（4E も同じ）。セキュリティビットやロックを立てる経路は無いので、SWD があれば必ず戻せる。
- **指摘**：S1 原子的でない（ベクタテーブルから上書きし、CRC は最後。電源断・ホストの中断・起動しないイメージで SWD 行き）／S2 イメージが基板用か確かめない（4E 用を 2E に書いても CRC が合えば「成功」。2E でもプレーン1宛てを受け付ける）／
  S3 検証前でもリセットできる／S4 エラッタ 46.1.1.3（書き込み中は FWS=6）を守っていない／S5 エラー後の再試行で FIFO の残りと FSR の読み捨て／S6 2E でも起動時に存在しない EFC1 に書いている（推測）／S7 コメントの誤り（SAM3U の IAP は文書化されている）。
- **提案**：A = すぐ入れられる対策（型番 CHIPID の照合、ベクタの検査、検証後のみリセット、FWS=6、FIFO と FSR の処理）。
  B（推奨）= 先頭に常駐ブート領域（ベクタ、クロック、LZMA、PS ローダ、ビットストリーム、更新プログラム、アプリの CRC 検査）を置いてロックビットで保護。2E で約 12 KB の余裕が残る見込み。
  ただし AG16K のビットストリームが 55〜75 KB になると入らない（上限 約 60 KB）。C = A/B の2面持ちは 2E では不可能（4E なら GPNVM2 で可能）。
- **追加の発見**：更新イメージを作って送るホスト側ツールがリポジトリに無く、イメージ形式（"OGFW"）も未定義。
- **判断（Opus）**：対策 A を入れるまで、**2E で `0xF9` による更新は使わない**（PLAN に登録）。B は G1 のビットストリームの大きさが分かってから決める。

### R38 G2 推奨 F6〜F8 と F13 の機能ビット（Sonnet 実装、Opus がテスト・ビルドを確認）
- `common/discimg::wsession`（状態遷移、保護範囲、FNV-1a、STATUS の符号化、パーティション開始の解析。テスト 16 件）、`fat.rs` の `partition_start`、`ide.rs` の配線。
- **コマンド**（プライマリへの `0xE0`）：6 BEGIN（`packet[2..6]="NGWS"`。LBA 0 が読めなければ拒否）／7 END（`packet[2]` = 0 空のまま＋UA、1 `load_media`、2 MOUNT_NAME で指定した名前）／
  8 STATUS（48 バイト：`"NGST"`、フラグ、受理セクタ数、失敗コマンド数、最後の書き込みの LBA・セクタ数・FNV-1a、保護境界）。
- F7：セッション中は第1パーティションより前（境界をまたぐものも）への書き込みを ABRT。F8：書き込みコマンドごとに FIFO から取り出した 512 バイトブロックの FNV-1a（F4 の分割をまたいで1つの値）。
- セッション中は START STOP UNIT のロード、IMAGE_CTL MOUNT、`0x52`、`0xF9` を拒否。ハードリセット（RSTn）でセッションを終えて再マウント、SRST では終えない。F13：機能バイト 0x07。
- **DESIGN-GAP**：二重 BEGIN は再開扱い、END だけは ILLEGAL REQUEST、名前なしの END 2 は 1 と同じ、LBA 0 が読めないと BEGIN を拒否、END 0 でも UA、タイムアウトなし など（`ide.rs` にコメント）。
  → 受信アプリ（`netgd-recv`、実装中）の Sonnet に形式を伝えた。
- **確認（Opus）**：discimg 87・gamecfg 21・lzma 5 件パス、4E・2E とも警告は従来の 13 件。2E の text 117,008（空き 約 13 KB）。IDE のホットパスのフレームは不変。

### R39 G1：FIFO をブロック RAM に乗る形に（Fable 実装、Opus が確認）
- `fpga/amaranth/hw/fifo.py`：`SyncReadFIFO`（登録読み出しのメモリ上の先読み FIFO。読み出しアドレスを1サイクル前に決め、同じアドレスへの書き込みは 17 ビットのバイパスで補う）と `make_fifo`。
  `--target cyclone2`（既定）は従来の `ResetInserter(SyncFIFO)` をそのまま、`--target ag16k` は `SyncReadFIFO`。AG16K 向けは `out/OpenGDEMUCore_ag16k.v` に出力。
- **確認（Opus）**：既定（v5.5）の Verilog はコミット済みのものと**バイト一致**、ag16k の生成は決定的、ag16k の `test_top`・`test_timing` がパス。
- **Fable の検証**：両ターゲットで 30/3/5/5 件パス。`test_fifo.py`（10 件）で旧 FIFO と1サイクル単位の一致（7 種類の深さで 2〜6 万サイクルのランダム試験、コア全体を2つ並べて 6 万サイクル×3）。
  バイパスを外すと即座に失敗することも確認（ミューテーション試験）。
- **資源**（YoWASP Yosys 0.71、Cyclone IV の規則）：LUT 97,839 → **2,821**（AG16K の 17 %）、FF 103,924 → **1,097**、ブロック RAM 0/5 → **5/5（14 M9K、25 %）**。
  代償は FF +85、LUT 約 +170。Yosys の M9K の techmap が途中で止まるため、その段だけ飛ばして集計した（スクリプトはスクラッチパッド）。
- **残るリスク**：AGM の `synth_agm` で同じように推論されるかは未確認（初回の Supra で `alta_bram9k` が 14 個あるか見る）、メモリの初期化、タイミング未解析。
- CI（`build.yaml`）に ag16k のシミュ、`test_fifo.py`、ag16k の Verilog 一致検査を追加。

### R40 吸い出しツール `tools/fwbackup` の実装と安全レビュー（Sonnet 実装、Fable レビュー、修正を Sonnet に依頼中）
- **実装**：`check`（CIDR/EXID で分類、終了コードで区別）、`dump`（2E かつロックなしのときだけ、4 KB × 32 チャンクを2回読み、食い違いは再読して多数決、上書きしない、`.bin`・`.sha256`・`.json`）、
  `inspect`（ベクタ、空ページ、文字列の数など）。probe-rs は `list`・`info`・`read` だけを許可するラッパー経由、`PROBE_RS_*` の環境変数を除去。テスト 37 件。
- **Fable のレビュー（probe-rs v0.32.0 と master のソースで確認）**：
  - **消去・書き込み・GPNVM・リセットに至る経路は無い**：`--connect-under-reset` なしの接続はリセットを出さない、SAM3U にはベンダー独自の unlock／消去シーケンスが無い、`read` はフラッシュアルゴリズムを走らせない、`list` はターゲットに触れない。
  - **高**：probe-rs はカレント・実行ファイルの場所・ホームの `.probe-rs.{toml,json,yaml,yml}` を読み、`[presets.default]` があると勝手にオプションを足す（環境変数の除去では防げない）。
  - **中**：「read はコアを止めない」は誤り。接続のたびに数 ms の halt が2回（`Session::new` → `clear_all_hw_breakpoints`）、再開時に1命令ステップ。dump 全体で約 260 回。リセット・フラッシュ・EEFC・RAM への書き込みは無い。
    → Dreamcast は BIOS メニューなどディスクアクセスの無い状態で実行する。
  - **中**：非 verbose の `info` へのフォールバックは自動検出の接続になる／probe-rs の版を固定していない／ロックされた SAM3U は DP ごと応答せず「応答なし」に見える可能性。
  - **低**：テスト用の JSON 形式のコマンド指定、値の検証の甘さ、書き込み途中の失敗で完成名のファイルが残りうる、など。
  - **判定**：`check` は条件付き GO、`dump` は指摘1〜3を直してから。条件 ― probe-rs は 0.32.0 ちょうど、設定ファイルが無いこと、配線は SWDIO/SWCLK/GND の3本だけ、BIOS メニューで実行。
  - 排除できないリスク：純正ファームがデバッガの接続を検知して自分でロックや消去をする作りなら、どの SWD ツールでも防げない（そういう証拠は無い）。
- 指摘1〜10 の修正を Sonnet に依頼した。修正が終わるまで fwbackup は使わない。

### R41 同じ製品の写真（ユーザーが共有、Opus が読解）
- ユーザーが使っているのと同じ GDEMU 6.0 の写真。U1：Atmel ATSAM3U2E AU A（2527、2527D7K）。U2：AGM **AG16KDDF256**、2行目 `EP6669-1`、日付コード `1921`（R6 の「PP6669」は誤読の可能性）。
- **U2 の型番末尾に「M」は無い** → G1 設計（R32）の「M の無い品は内蔵フラッシュを持たない可能性」と、外付けフラッシュが無いこと（R6）から、
  **MCU が FPGA をパッシブシリアルで起動する方式（分岐 A）がさらに有力**。確定は作業2の導通確認（PA23/24/25・PB1/2 が U2 の方へ向かうか）。
- ヘッダの配置（MCU 横 3 ピン、下辺 5 ピン、SD 横の縦 8 ピン、右上 2 ピン）、ボタン、LED（D1）の位置も手元の基板と同じ。
- 画像ファイルはリポジトリに保存していない（会話内のみ）。基板メモ `docs/boards/gdemu60/README.md` のチェックリストを更新した。

### R42 SWD プローブに ESP32 は使えるか（ユーザーの質問、Opus が回答）
- **調べたこと**（web-fetch、各 README）：windowsair/wireless-esp8266-dap は ESP8266/ESP32/ESP32-C3/ESP32-S3 で CMSIS-DAP（SWD/JTAG）を提供するが、**PC との接続は Wi-Fi のみ**（USBIP で仮想 USB、または Keil 用の elaphureLink）。
  OpenOCD・pyOCD・Keil は対応と明記、**probe-rs は記載なし**。Espressif の esp-usb-bridge は ESP32-S2/S3 の本体 USB で CMSIS-DAP（バルク転送のみ）の SWD を提供（Espressif 以外のチップでの SWD は README で明言なし）。
- **ユーザーの手持ち**（写真）：無印の **ESP32**（「ESP-32」モジュール、30 ピン DevKit、USB-シリアル変換チップ経由、裏面「ESP32 ESP32S 30P Expansion board」）。ほかに SPI の microSD モジュール。
- **判断**：無印 ESP32 は本体に USB が無く esp-usb-bridge が使えない。残る wireless-esp8266-dap は無線経由で probe-rs 非対応、fwbackup の安全レビュー（probe-rs 0.32.0 ＋ USB の CMSIS-DAP）の範囲外 → **使わない**。
  **Raspberry Pi Pico（または Pico 2、公式 Debug Probe）を用意する**。ESP32-S3/S2 なら esp-usb-bridge で使える見込みはある。

### R43 `tools/fwbackup` の安全修正（Sonnet、Opus がテストを確認）→ 使用可
- R40 の指摘 1〜10 をすべて反映：probe-rs の設定ファイル（`.probe-rs.{toml,json,yaml,yml}`、カレント・ホーム・%USERPROFILE%・実行ファイルの場所）があれば何も起動せずに拒否し、probe-rs は空の一時ディレクトリをカレントにして起動／
  「read は接続ごとに数 ms halt する」と正しく記述（BIOS メニューで実行）／古い probe-rs 向けの自動検出フォールバックを削除／**probe-rs は 0.32.0 ちょうどだけ許可**（上書き手段なし）／
  「応答なし」でもロックの可能性があると明記／テスト用の JSON 形式は `FWBACKUP_TEST_WRAPPER=1` のときだけ／値の検証を厳格化／`.bin` は `.part` に書いて fsync・再ハッシュしてから改名（上書きしない）／
  全プローブが使用不可なら NO_PROBE、`--chunk` の事前検証、2E の別リビジョンは別表示（dump は拒否）、JSON に実際のプローブを記録。README に条件（0.32.0、設定ファイルなし、3本の配線、BIOS メニュー、改名しない、クラウド同期フォルダに置かない、法的助言ではない、ロックを破らない、GPNVM などは含まない、`download`・`erase`・`reset` を手で使わない）。
- **確認（Opus）**：テスト 59 件パス、`.gitignore` が `.bin`・`.part`・`gdemu60_flash_*` を除外することを `git check-ignore` で確認。
- これで **fwbackup は使用可**（`check` → ロックなし 2E なら `dump`）。実機と実物の probe-rs では未検証。

### R44 今日の残り作業の割り当てと、Pico の配線をガイドに追加（Opus）
- 割り当て：Sonnet = 自己更新（`0xF9`）の安全対策 A と更新イメージのツール `tools/fwimage`／Fable = GDEMU.INI・CDI 修正・F6〜F8 のデバッグ（読むだけ）／
  Haiku = README と資料の索引を新しいツールに合わせる、`ide_device.py` の古いコメント／Opus = Pico の配線。改名（#7）は `mcu/` を触る作業と `netgd-recv` の後。
- **Pico の配線**（実機作業ガイド 版10、`INSTALL.md` §3 と同じ）：GP2（物理ピン4）= SWCLK → SAM3U 9 番、GP3（物理ピン5）= SWDIO → SAM3U 7 番、GND（物理ピン3 など）→ ヘッダの GND。
  3V3・VSYS・VBUS はつながない（電源は Dreamcast から）。**6.0 のヘッダは穴だけでピンが無い** → ピンヘッダのはんだ付けを推奨。信号は双方 3.3 V。
- ガイドの作業3に `fwbackup` の手順（`check` → ロックなし 2E なら `dump` → `inspect`）と条件を追加。

### R45 GDEMU.INI・CDI 修正・F6〜F8 のデバッグ（Fable、読むだけ）
- 重大な欠陥なし。中〜低 9 件（テスト・ビルドは HEAD の別作業ツリーで確認、4E・2E とも成功）：
  1. **（中）open/detect の NOT READY の窓が TUR・CD_PLAY・CD_READ にしか効かない**。GET_TOC・REQ_SES・GET_SCD・REQ_STAT は通り、SECNR も STANDBY のまま。UA は窓の前に1回だけ、CD_READ の経路はセンスを設定せず、
     REQ_ERROR（0x13）は常にゼロ。窓が付くのは `mount_named` 経由だけで、`swap_media`／`load_media` 経由は窓なし。→ openMenu の `cdrom_reinit` が窓の中で OK になり、直後の CD_READ が失敗しうる（INI にキーがあるときだけ）。
  2. **（中）GDEMU.INI の読み込み**：512 バイトで黙って切れる（値の途中で切れると `reset_goto=12` が 1 になる）、UTF-8 BOM で1行目が未知キー、**Shift-JIS のコメント以降が全部捨てられる**、CR だけの改行も不可（すべてテストで確認）。
  3. （中〜低）`reset_goto` と END 2 のマウントが古い INI を使う（`mount_named` 経路は INI を読み直さない）。
  4. （低〜中）END 2 が `0x52` の残した古い `MOUNT_NAME` をマウントしうる（使った後も BEGIN でも消さない）。
  5. （低）CDI：`start_lba + pregap < 150` のとき LBA だけ 0 に張り付き、ファイル位置がずれる（壊れた／特殊なイメージのみ）。
  6. （低）F8/STATUS：`write_begin` より前で拒否された書き込みが記録されず、STATUS が直前の成功を返す。
  7. （低）ハードリセットで再マウントしない経路では `READY_AT` が残り、起動中の BIOS に NOT READY が返り続けうる。
  8. （記録の訂正）R36 の「lead-out は変更なし」は不正確：`start_lba` の意味が変わったので、最終トラックの pregap が 150 以外だと lead-out と REQ_SES(0) が `150 - pregap` だけ前に動く。Flycast との関係は pregap 150 のときと同じ（+1）で、方向としては改善。
  9. （低）保護境界は MBR のエントリ 0 を無条件に信じる（GPT の保護 MBR では守れない）→ 種別 0xEE なら BEGIN を拒否。
- **判断（Opus）**：1 は、純正のプローブ記録（切替後の SENSE の並びと時間）が取れるまで**窓を既定で無効にする**（INI の値は読むが適用しない）。中途半端に効くより安全。
  2〜7・9 は Sonnet に修正させる（`ide.rs` の自己更新部分を編集中の作業が終わってから）。8 は LOG の訂正のみ。

### R46 自己更新の安全対策 A と更新イメージのツール `tools/fwimage`（Sonnet 実装、Opus がテスト・ビルドを確認）
- **OGFW 形式 v2**（`tools/fwimage/FORMAT.md`）：512 バイトのヘッダ（magic、形式 2、種別、**対象の CHIPID**、宛先、長さ、ペイロード CRC32、版、ヘッダ CRC32）＋ペイロード。2E はコード1つ（0x80000）、4E はコード＋ビットストリーム（0x100000）。
- **対策 A**：CHIPID の照合（違えば BadTarget）、4E 以外のプレーン1は BadDest／先頭 256 バイトを RAM に受けてベクタを検査してから書き始め、**ページ0は最後**（残りの CRC を確かめてから書く）／
  リセットは検証が通った後だけ（強制リセットは別の鍵）／書き込み中 FWS=6（エラッタ）、EFC1 は 4E のときだけ／エラー時に FIFO を空読み、FCR の前に FSR を読み捨て、GLB でロックを確認。
  `embassy-atsam3` に `sam3u2e` feature を追加して 2E では EFC1 に触れない。更新プログラムは 888 → 2,188 バイト（RAM 上のみで動くことを逆アセンブルで確認）。2E の text 118,304（空き 約 13 KB）。
- **確認（Opus）**：discimg 87・gamecfg 21・lzma 5、fwupdater の純粋ロジック 5、`fwimage` 17 件（実際の 4E・2E の ELF を使用）パス。4E・2E とも警告は従来の 13 件。
- **残る窓**：転送中（ページ1以降を消してからページ0を書き直すまで）の電源断、ページ0の書き込みの数 ms、中断した転送、強制リセット。解消には対策 B（ロックで守る常駐ブート領域）が必要。
  レジスタを叩く状態遷移そのものにはホストテストが無い。BBA 経由の送信側は未作成（`iter_images()` が将来の送信側の入口）。

### R47 追加の並列割り当て（Opus、ユーザーの指示「他に進められることがあれば役割を与えて」）
| モデル | 作業 | 担当ファイル |
|---|---|---|
| Sonnet | BBA 経由のファーム更新アプリ `netgd-fwupd`（`fwimage` の OGFW を `0xF9` でセカンダリへ送る KOS アプリ、偽デバイスでホストテスト） | `tools/netgd-fwupd/` |
| Sonnet | プローブに CD_READ の読み比べ試験（CDB バイト1 = 0x20/0x24/0x28/0x10、R24 の提案） | `tools/gdprobe/` |
| Fable | AG16K のコンフィグ方式（パッシブシリアル）の詳細調査と `cyclone.rs` の変更設計 | `docs/research/ag16k-config.md` |
| Haiku | CI に Python ツールのテスト（mksdcard・netgd-send・fwbackup・fwimage・netgd-recv）と fwupdater のホストテストを追加 | `.github/workflows/build.yaml` |
- 並行して：Sonnet = R45 の修正（`ide.rs`・`gdi.rs`・`gamecfg`・`discimg`）、Fable = `netgd-recv` のレビュー、Opus = `netgd-recv` のテストの再実行とコミット。
- 改名（`opengdemu` → `netgd`）は `mcu/` を触る作業がすべて終わってから。

### R48 `tools/netgd-recv` の実装（Sonnet）とレビュー（Fable）
- **実装**：KOS アプリ（`main.c`・`plat_kos.c`・`dev_kos.c`・`fwctl.c`）＋ホストでテストできる中核（`ngblk`・`ngfs`・`recv_core`・`xport_pc`・`manifest`・`crc32`・`diskio`）、FatFs R0.15b を同梱（ライセンス保持）。
  F6 のセッションを開いて連続領域に書き、書き込みごとに STATUS の FNV-1a と照合（不一致は最大3回書き直し）、8 MiB ごとのチェックポイント、最後に1セクタの改名で公開、セッションを閉じて再マウント。
  ホストテスト 14 件（**Opus が再実行してパス**）：通常の書き込み（高速・低速の両経路）、既存・保護フォルダ、ENOSPC でカードが1バイトも変わらない、断片化、ビット化けの書き直し、中断と再開、
  **全書き込み位置での電源断**（新旧どちらかの完全なイメージしか見えず、FAT も壊れない）。逸脱 15 件は README に列挙。
- **Fable のレビュー**：**FAT やイメージを壊す欠陥はなし。捨ててよい SD カードと caps=7 のファームなら初回の実機試験は GO**。
  1. （中）**前回の実行で残った UA に VERSION が食われ、2回に1回「NetGD ではない」と誤判定**（`0xE0` は UA の除外対象でない）。END 2 失敗後の `MOUNT 0xFF` の保険も同じ理由で効かない → CHECK でセンスキー 6 なら1回だけ再送。
  2. （中）**BEGIN 後のエラー経路（FAT32 でない等）で END を出さず**、取り出し・ボタン無効のまま戻る。
  3. （中）`delete`・`verify`・`finish mount` の引数を検証せず、`/01`・`01.`・`01 ` などで保護フォルダの判定をすり抜けられる（FatFs は末尾の空白と `.` を落とす）。
  4. （中、推測）FAT・ディレクトリの**読み込み時**のビット化けは検出されない（書き込み側の FNV は「化けたとおりに書けた」と判定）→ 1セクタ読みは2回読んで一致確認。
  5. （中〜低）確保中の電源断でクラスタが漏れ、FSInfo の空き数が過大になる → `FF_FS_NOFSINFO=1` か、README に chkdsk の案内。
  6. （低〜中）KOS の g1ata は PIO の書き込みエラーを返さない → STATUS の照合に頼るので、`FW_CAP_SESSION | FW_CAP_CHECKSUM` が無いファームでは書かずに止める。
  7. （低）`atapi()` の後で SLAVE を選ぶと、ファームが無応答のとき無限待ちになりうる → その行は不要（unlock が MASTER を選ぶ）。
  8. （低）ウォッチドッグの 64 ビット値の読み取りが非アトミック。9.（低）読み戻し中は中断確認も進捗も無い。10.（低）直接書きの前提（クラスタ範囲・連続性）を検査していない。11.（低）その他（`list` の打ち切り、96 文字超の名前、記述子名との衝突、容量の +1、DMA の自動フォールバックなし）。
  - 実機で見る点：KOS の PIO 書き込みはセクタ間で BSY しか待たず、FIFO があふれうる → `fnv_mismatches` を必ず確認。
- **対応**：1〜11 を Sonnet に修正させる（`tools/netgd-recv` のみ）。

### R49 AG16K のコンフィグ方式の調査（Fable）
- **詳細**：`docs/research/ag16k-config.md`。
- **要点**：
  1. **「M」の無い AG16KDDF256 は FPGA ＋ 積層 DDR だけで、内蔵フラッシュは無い**（AGM の DDF256 マニュアル、一次資料。ただし「フラッシュなし」と明記した一文は無い）。AS は外付け SPI フラッシュ前提で、基板には無い
     → 分岐 B1 はほぼ消え、**分岐 A（MCU がパッシブシリアルで起動）がほぼ確実**。MCU が JTAG で起動する A'' もありうる。
  2. モードは MSEL1 だけで選ぶ（0 = PS、1 = AS。JTAG はどちらでも可）。
  3. コンフィグ用ボール（マニュアルのボール配置図から目視）：DCLK=G1、DATA0=G2、NCONFIG=H3、NSTATUS=F3、CONF_DONE=F16、MSEL1=F15、TDI/TCK/TMS/TDO=H1/H2/J1/J2。nCE・INIT_DONE は無い。
     → **`g1-ag16k-port.md` §2-4 の推定（SDF256 / EP4CE15F17 の配置）は DDF256 には当てはまらない**（訂正）。資料は CONFIDENTIAL の透かし入りなので対応表だけを記録。
  4. プルアップ：NCONFIG 10 kΩ、NSTATUS 10 kΩ、CONF_DONE 1 kΩ。I/O バンク 4 つ（3.3/2.5/1.8 V）、DDR 用に 2.5 V と 1.25 V。
  5. PS で流すのは `.bin` か `.rbf`（`.rbf` は各バイトのビット順を逆にしたもの、一次資料）。線上は `.bin` の MSB 先頭＝`.rbf` の LSB 先頭と推測 → 今の LSB 先頭のローダーで `.rbf` をそのまま流せる見込み（外れても CONF_DONE が上がらないだけ）。
  6. **AG16K は圧縮（LZW）ビットストリームを PS で受け付ける**（一次資料）。NetGD の設計の圧縮後サイズは Supra でビルドするまで不明。LZW 済みは LZMA でほとんど縮まないので、フラッシュの大きさと読み込み時間はトレードオフ。
  7. **PS のタイミング表・波形・CONF_DONE 後のクロック数は AGM の資料に無い**。Cyclone IV E の値（記憶に基づく）に 10 倍以上の余裕を持たせて始め、実装前に CYIV-51008 で確認する。
  8. 読み込み時間の見積もり（552 KB = 4,418,928 ビット、MCU 96 MHz）：今のループで 2.2〜3.3 秒、NOP を除けば 0.7〜1.2 秒、1ビットあたりポート書き込み2回なら 0.5〜0.9 秒。
  9. ローダーの変更設計（§6）：`PsProfile`（ビット順、タイムアウト、CONF_DONE 後に 64 クロック、パディングの上限）、`panic!` をやめて `Result` と最大3回の再試行、`build.rs` でボードごとに `.rbf` を選ぶ。
- **Opus の注記**：Fable が最も安価な確認として挙げた「F15（MSEL1）が GND か」は、**BGA の下のボールなのでテスターでは直接当てられない**（近くにビアが出ていれば可能）。
  現実的な確認は、作業2の導通確認で MCU の PA24/PA23/PA25/PB2/PB1 が FPGA の方へ伸びているかを見ること。決め手は、純正の電源投入時の DCLK/DATA0 をロジアナで見ること（ビット数・ビット順・追加クロック・読み込み時間が全部分かる）。
  AG16K のオープンな JTAG ツールや、AGM 用の MCU からの PS ローダーの公開例は見つからなかった。

### R50 CI の拡充と、Linux で初めて分かった不具合（Opus）
- `build.yaml` に `tools-host` ジョブ（mksdcard・netgd-send・fwbackup・fwimage・netgd-recv のホストテスト）と fwupdater のホストテストを追加（Haiku）、プローブに CD_READ の読み比べ試験を追加（Sonnet）。
- **初回の実行（run 44685118507）**：ファーム・ホストテスト・gdprobe・FPGA シミュ・fwbackup・fwimage・mksdcard は成功。**失敗が2つ**：
  - `netgd-send`：UNC パスのテストは Windows でしか成り立たない → Windows のときだけ試すよう修正（Opus、`5f4e02d8`）。
  - `netgd-recv`：ホストテストのコードが Windows 専用（`direct.h`）で、`size_t` のヘッダも不足 → Linux でビルドできない。`netgd-recv` の修正を担当している Sonnet に追加で依頼。
- **KOS ビルドの CI も失敗**（run 44684499681）：`netgd-recv` の Makefile の文字列の `-D` 定義の引用符が崩れる（KOS の Makefile.rules 経由）、`%u` と `long` の不一致 → 同じ Sonnet に依頼（gdprobe と同じく、引用符なしで渡して C 側で文字列化する方式に）。
- 教訓：Windows だけでテストしていると Linux や KOS の環境差を見逃す → CI で両方を回す意味があった。

### R51 さらに追加の並列割り当て（Opus、ユーザーの指示）
| モデル | 作業 | 担当ファイル |
|---|---|---|
| Sonnet | G1：ボード対応の FPGA ローダー（`PsProfile`、再試行、タイムアウト、`board-v55`／`board-v60`）。v5.5 の動作は変えない。偽 FPGA でのホストテストは新クレート `common/psload` | `cyclone.rs`・`main.rs`（FPGA 部分）・`build.rs`・`Cargo.toml` の feature・`common/psload/`・CI |
| Sonnet | 「NetGD ならこう答える」予測記録ツール（mksdcard のイメージに対する NetGD の応答をプローブ形式で出力。純正の記録とすぐ比較できる） | `common/discimg/examples/expect.rs`、`tools/netgd-expect/` |
| Fable | 自己更新の対策 B（ロックで守る常駐ブート領域）の設計（2E/4E の配置、ビットストリームの3つの想定、SD から読む案） | `docs/design/fw-bootloader.md` |
| Haiku | 計画書の「いまの状態」を最新化 | `docs/PLAN.md` の該当節 |
- 実行中の作業：`netgd-recv` の修正、R45 の修正、`netgd-fwupd`（いずれも Sonnet）。

### R52 BBA 経由のファーム更新アプリ `tools/netgd-fwupd`（Sonnet 実装、Opus がテストを確認）
- KOS アプリ（`main.c`・`g1dev.c`）＋ホストでテストできる中核（`fwcore.c`：OGFW の検査と `0xF9` の状態遷移、結果の文章）、結果ファイルは 4 KB ずつ＋flush。
- **手順**（`fwupdate.rs`・`fwupdater/main.rs`・`ide.rs` の `handle_fw_command` に準拠）：セカンダリの IDENTIFY の型番に「OpenGDEMU」か「NetGD」が含まれるときだけ進む（**純正 GDEMU にはセカンダリが無いので何も送らない**）→
  画面で確認し A+X+START+上 を3秒長押し → arm（feat 0x4F）→ enter（0x53）→ イメージごとに header（0x55）・data（0x56、64 ブロックずつ）・verify（0x57）→ 全部が検証済みのときだけ reset（0x58）。
  **強制リセット 0x59 は定数すら無く、送らないことをテストで確認**。再試行は Overrun・FlashError・CRC 不一致などだけ（最大2回、ヘッダから）。BadTarget・BadVector・Locked・タイムアウトは再試行しない。
- 追加：前回の失敗で RAM 上の更新プログラムが残っている場合を検出して続きから送る（arm が拒否されたら verify を送って応答で判別）。完全で未確定のイメージがあれば、その確認で確定する（副作用）。
- **確認（Opus）**：テスト 872 項目パス（共有のテストベクタ、偽の更新プログラムでの成功・各エラー・タイムアウト・再開、2E/4E の実イメージ）。KOS 部分は構文チェックのみ（KOS CI で初コンパイル）。
- **未確認**：実機との PIO の手順、enter 直後の状態の取り方、IDENTIFY の型番の判定（改名で型番を変えたら要更新）。転送中の電源断の窓は R46 のとおり残る。

### R53 R45 の修正（Sonnet、Opus がテスト・ビルドを確認）
- open/detect の窓は既定で無効（`APPLY_SWAP_WINDOW = false`、再び有効にする条件をコメントに列挙）、ハードリセットで窓を消す／GDEMU.INI をバイト列で行ごとに解析（CR/LF/CRLF、BOM、512 バイト超はバッファを使い回して全部読む、非 ASCII の行だけ無視）／
  `reset_goto` と END の再マウントの前に INI を読み直す／`MOUNT_NAME` を使うたびに消す／CDI で FAD 150 より前に始まるトラックは足りない分を飛ばす（全部が下なら BadCdi）／
  CMD25 より前に拒否した書き込みも STATUS に失敗として記録／GPT の保護 MBR なら BEGIN を拒否。
- **確認（Opus）**：discimg 89・gamecfg 28・lzma 5 件パス、4E・2E とも警告は従来の 13 件、2E の text 118,312。コミット `16c4fdf5`。

### R54 自己更新の対策 B の設計（Fable）
- **詳細**：`docs/design/fw-bootloader.md`。
- **結論**：R37 の案（ビットストリームをロック領域に置き、復旧は `0xF9` だけ）は**勧めない**。電源を入れ直すと Dreamcast は `netgd-fwupd` を読み込めない（dcload のディスクを NetGD 自身が提供しているため）。`0xF9` だけの更新モードは送信側が RAM にいる間しか役に立たず、それは対策 A で足りている。
- **推奨「B-SD」**：0x80000 の 8 KB（ロック領域0）に常駐部（クロック、アプリの記述子・CRC・ベクタの検査、VTOR でジャンプ）＋ **SD からの復旧**（カードのルートの `NETGD.OGF` をポーリングの HSMCI と読み取り専用の FAT32 で読む。FPGA・Dreamcast・LAN は不要）。
  起動条件：アプリが無効、電源投入時にボタン、アプリからの GPBR 要求。消去の前にファイルを完全に検査し、アプリの先頭2ページを最初に消して最後に書く → 電源断でも再び復旧に入るだけ。
- **試作で計測したサイズ**（スクラッチパッドの試作、未実行）：常駐の基本 1,072 B、＋SD 復旧 4,308 B（8 KB に収まり 約 3.5 KB 余る）、＋FPGA ローダー/LZMA なら 7,868 B（8 KB ではきつく 16 KB が必要）。
- **2E の配置**（アプリは 0x82000 から 122,880 B）：Cyclone II（49.6 KB）埋め込みで残り 約 4.1 KB、AG16K 小（40 KB 以下）埋め込みで 約 13.9 KB 以上、AG16K 大（55〜75 KB）は SD の `NETGD.BIT` をアプリが読む形で 約 53 KB。
  埋め込める上限は 8 KB の予備を残すと約 46 KB。4E は常駐 8 KB＋アプリをプレーン0、ビットストリームはプレーン1。
- アプリは自己完結のまま（ローダー・LZMA・ビットストリーム・RAM 更新プログラムは今のまま）、`"NGAP"` 記述子と CRC トレーラを付けてオフセットでリンク。OGFW 形式 3。FPGA の起動が CRC 検査のぶん 約 20〜55 ms 遅れる（見積もり）。
- ロック：常駐部が起動時に自分の領域をロック（CLB はどこにも書かない）。EEFC コマンドは EWP・GLB・（常駐部だけ）SLB の許可リストに限定。今のページ0の EWP の語 `0x5A000003` はセキュリティビットの語 `0x5A00000B` と1ビット違いだが、B では EWP の引数が常に 32 以上なのでその語は作られない。
- B の後も SWD が必要なもの：常駐部の書き換え・不具合、最初の書き込み、ボタンが使えない基板で CRC は正しいが動かないアプリ。A で残っていた窓（転送の中断、ページ0の書き込み、強制リセット、ビットストリームの更新失敗）は SD で復旧できるようになる。
- **オーナーの判断事項**：復旧経路（SD だけを推奨）、8 KB か 16 KB か、今の形式2の更新プログラムで SWD なしに移行するイメージを許すか、自己ロックか SWD 時のロックか、更新のきっかけ、ファイル名と LED の表示（ロック後は変えられない）。
- **実機での確認が必要**：VTOR を移した embassy の割り込み、オーナーのカードでのポーリングの SD 初期化、ロックが EWP を止めつつ SWD で確実に書き直せること、FPGA の遅れを Dreamcast が許すか。

### R55 予測記録ツール `tools/netgd-expect`（Sonnet、Opus が compare のテストを確認）
- `common/discimg/examples/expect.rs`：GDI/CDI をファームと同じ方法で読み、`atapi` の応答ビルダと `ide.rs`（HEAD）の包み方（ステータス、UA、REQ_ERROR は常にゼロ、REQ_MODE、`52 00`、CD_READ はバイト1を無視して 2048 バイト）で、
  プローブと同じ形式の記録を出す。時間に関わる値は `-`。`tools/netgd-expect/expect_sd.py` は mksdcard の出力全体を、プローブの切替実行と同じ形の1つの記録にする（`07` は ILLEGAL REQUEST でディスクはそのまま）。
- 検証（Sonnet）：予測記録どうしの比較で差 0、`--manifest` で 4,172 一致・0 不一致、テスト 10 件。`compare.py` に `--only-common`（片方にしか無い項目を無視）を追加（既定の動作は不変、Opus がテストを再実行して確認）。
- 使い方：純正の記録が届いたら `compare.py --only-common <純正> <予測>` → 差分がそのまま D-1〜D-5 などの判断材料になる（README に対応表）。
- 前提：UA の出方、切替後2回目の TUR で READY、ABRT 時の irr など、HEAD の `ide.rs` の読解に基づく。`ide.rs` が変わったら例の中の定数（版 5.20.0、REQ_MODE）を更新する。

### R56 対策 B の方針決定と S0〜S2 の割り当て
- ユーザーが Opus の推奨（下の判断の表 D1〜D7）を受け入れ、実機の要らない段階 S0〜S2 の並列実施を指示。
- 割り当て：Sonnet = **S0** `mcu/bootcore`（純粋ロジック、FAT32 リーダー、形式3の解析、SD 復旧の状態遷移、ロックビットと「書き込み中は読めない」を持つフラッシュの模型で電源断を総当たり）／
  Sonnet = **S1** OGFW 形式3（`fwimage`・`fwupdater` の `logic.rs`・`netgd-fwupd` の `fwcore.c` に共通ベクタで。今の基板は形式2のまま、形式3は feature `resident` のときだけ）。
- **S2（アプリを 0x82000 にリンク、VTOR、クロック初期化の冪等化、CI で両配置）は、FPGA ローダーの作業（`main.rs`・`Cargo.toml`・`build.rs`・CI を編集中）が終わってから**割り当てる（ファイルが重なるため）。

### R57 基板ごとの FPGA ローダー `common/psload`（Sonnet 実装、Opus がテストと3種のビルドで確認）
- 問い：AG16K（6.0）でも v5.5 の Cyclone II でも同じコードで FPGA を設定できるか。電源が入らない FPGA で固まらないか。
- 結論：受動シリアルの手順を純粋ロジックのクレート `common/psload` に切り出し、ピンと時間はトレイト（`PsPins`・`PsClock`）で渡す。基板ごとの差は `PsProfile`（タイミング・ビット順）。
  タイムアウトでエラーを返し、再試行の方針もクレート側。`cyclone.rs` はこれを呼ぶだけになった。
- feature：`board-v55`（既定）／`board-v60`（= `sam3u2e`、ビットストリームは任意の `fpga/agm/output/OpenGDEMU_ag16k.rbf`、無ければビルドは通り起動時に `NoBitstream`、LED 5回点滅）／`ps-msb-first`（AG16K のビット順は推測のため切替用）。
- 根拠：`common/psload/src/lib.rs`、`tests/equivalence.rs`（旧ループを文字どおり移植して波形を比較）、`tests/sim.rs`。
  Opus の実行結果：`psload: 17 passed, 0 failed`、4E・2E・v60 のビルドすべて exit=0（警告 12 件は既存のもの）。
- 信頼度：ホストで確認済み。**6.0 のローダーのピン5本は v5.5 のものを仮に使っており未確認**（`cyclone::pinmap`）。実機では未検証。
- CI：`host-tests` に `common/psload` を追加。
- 関連：`mcu/opengdemu/src/cyclone.rs`・`main.rs`・`build.rs`・`Cargo.toml`、`docs/research/ag16k-config.md` §6。

### R58 S0：常駐部の純粋ロジック `mcu/bootcore`（Sonnet 実装、Opus は報告を確認、Fable のレビュー待ち）
- 問い：対策 B の常駐部（SD 復旧）のロジックを、実機なしで電源断まで含めて検証できるか。8 KB に収まるか。
- 結論：依存なしの `no_std` クレート。モジュールは `crc`（4ビット表 64 B、残差 `0x2144DF1C`）・`part`・`app`（`APP_BASE`、最大 0x1E000、記述子・トレーラ・ベクタ検査）・
  `fcr`（EEFC コマンドは EWP・GLB・SLB だけ。プレーン0の EWP はページ 32〜511、SLB は許可フラグ付きでページ 0〜31）・`flash`（トレイト）・`fat`（FAT32 リーダー、512 B バッファ1つ、Brent の循環検出）・
  `ogfw`（形式3専用、ヘッダ CRC はバイト 0..31 と 36..39）・`update`（`sd_recover`）・`boot`（`decide`、GPBR の合図 `"NGRC"`）。
- S1 との整合：`FORMAT.md` の形式3と共通ベクタ（`header3`・`desc`・`trailer`・`image3`・`vtable`・`crc32`）をすべて bootcore でも検査し一致。形式2の行はすべて `BadHeader`。
- テスト 54 件すべて通過（`basics` 19・`fat` 17・`update` 11・`power_cut` 7）。
  - `fcr_word` を全コマンド×引数で総当たり：プレーン0（SLB なし）で作れる語は 481 個（EWP 480＋GLB 1）だけで、SGPB・CGPB・CLB・EA・WP の語やセキュリティビットの語 `0x5A00000B` は作れない。
  - 電源断の総当たり：すべての EWP で「未開始・消去済み・k バイトまで書込み・消去途中・ゴミ」を試し、起動判定が受け入れるのは旧イメージか新イメージそのものだけ。2E 空フラッシュで 17,442 通り、新旧入れ替えで約 46,000 通りなど。再開時の二度目の断も 1,000 通り超。
  - SD の読み取り失敗・1バイト破損をすべての読み取り位置に入れても「正しく見えて中身が違うアプリ」は残らない。
- サイズ（thumbv7m、opt s、LTO、`sizeprobe`）：全体 約 5.4 KB（memcpy 等 1.2 KB を含む）、ロジック 約 4.0 KB。HSMCI の SD 読み取りと起動コードはこれに加わるが、8 KB に余裕あり（見積もり）。
- 実装上の判断：(1) 最初のチャンクを最後に書くため SD を3回目に読む、(2) `fcr_word` はプレーンを取る（EEFC1 はページ0の EWP が必要。EEFC1 に GPNVM は無いので害なし）、
  (3) ビットストリームだけのファイルは `NoApp` で拒否、(4) FAT は MBR の種別 0x0B/0x0C のみ、0xEE（GPT）は拒否、BPB の矛盾も拒否。
- 未確定：GPBR の合図 `"NGRC"` は Sonnet の提案、EWP とポーリングは実機では RAM から実行する必要がある（`sizeprobe` の実装は寸法用で未実行）、LED の表示は保留（D6）。
  テスト用の FAT32 イメージは自作の生成器によるもので、独立したフォーマッタとの照合はしていない。
- 信頼度：ホストテストで確認済み（Opus は報告とファイル構成を確認、テストの再実行は CI に任せる）。実機では未検証。
- CI：`host-tests` に `mcu/bootcore` を追加。
- 関連：`mcu/bootcore/`、`docs/design/fw-bootloader.md`、`tools/fwimage/FORMAT.md`。

### R59 S0 `bootcore` のレビュー（Fable、`cargo test` 54 件通過を再確認）
- 問い：常駐部のロジックに、常駐領域を書き換える経路や、壊れた SD のファイルで間違ったアプリを受け入れる経路がないか。
- 結論：ホストで再現できるそのような経路は**見つからず**。ただし、許可リストがまだ許しているもの、`Flash` トレイトと模型では表せないもの、復旧の繰り返しについて指摘がある。
- 指摘（重い順）：
  1. **必須（S3 の前）**：`Flash` トレイトが `ewp_start` と `ewp_poll` に分かれていて（`src/flash.rs:39-56`）、ポーリングのループが flash 上の bootcore のコードになる。2E ではプレーン0を書きながらプレーン0から命令を読むことになる。
     模型が数えるのは `Flash::read` のデータ読み取りだけなので、`violations == 0` は命令フェッチについて何も示さない。→ RAM 上の1つの関数で行う、ブロックする `ewp` だけにする（タイムアウト付き）。
  2. SLB の引数 0..31 を許している（`src/fcr.rs:117`）。データシートでは「ページ番号」とも「ロックビット番号」とも読める。後者ならアプリ領域がロックされ、CLB はどこにも無いので SWD を使うしかない。SLB 1 は CGPB(1) と1ビット違い。→ 引数 0 だけにする。
  3. GLB の語 `0x5A00000A` がセキュリティビットの語 `0x5A00000B` と1ビット違い（毎回の起動で発行する設計）。→ GLB には引数 3 以上の定数を入れ（例 `0x5A00200A`）、化けて SGPB になっても効かないようにする。距離のテストを GLB・SLB にも広げる。
  4. 復旧の要求を消すものがない（`src/boot.rs:22-28`）。GPBR0 はリセット後も残るので、復旧が成功してもリセットのたびに書き直し続ける（約 1 万回の書き換え寿命を消費し、アプリが起動しない）。ボタンが押しっぱなしでも同じ。
     → API で消す義務を表す。同じイメージなら書き込みを省く。真理値表のテストは同じ式を計算し直しているだけ。
  5. プレーンとレジスタの組み合わせを実装側に任せている（`src/fcr.rs:79-93`）。P1 の EWP 0..31 の語を EEFC0 に書けば常駐部自身のページになる。→ アドレスと語を組にした型にし、`Part` を受け取り、SLB は明示的に許可したときだけ。
  6. `erase_head` が消去後に 0xFF をそのまま要求している（`src/update.rs:171-175`）。弱いセル1つで復旧が毎回失敗する。→ 消去後に「記述子として読めない」ことを確かめれば足りる。
  7. FAT は自作の生成器のイメージでしかテストしていない（FAT の予約上位4ビット、32 GB 級の配置、FAT ミラーのフラグ、LBA0 が BPB に見える MBR、パーティションは最初の項目だけ）。拒否側に倒れるので誤書き込みにはならないが、ロック後は SWD でしか直せない。→ **ロックの前に** mksdcard・mkfs.fat・Windows のフォーマットのイメージで試す。
  8. 注記：2回目の読み取りパスではヘッダが同じかしか確かめない。ボタンで要求した復旧は、検証後にカードが壊れると有効な旧アプリを失う（設計どおりだが明記する）。
  9. 小さな点：FRDY が上がらないと `Flash::ewp` は永久に回る、`lock_mask_at` は `pub` で dest < 0x80000 だと下にあふれる、Brent の `steps += 1`（デバッグビルドだけ）。
- 実機で確かめる必要がある項目：
  - 正誤表 46.1.1.3 / 46.2.1.2：書き込み中は FWS = 6 が必要（設計にもコードにも書かれていない）。
  - FLOCKE・FCMDE は FSR を読むと消えるので、FRDY を見た同じ読み取りから取る。ラッチへの書き込みは 32 ビット単位だけ。
  - 4E の GPNVM2（0x80000 に割り当てるプレーンを入れ替える。許可リストに GGPB が無いので常駐部は確かめられない）。
  - 不安定なセル、未知の CHIPID の扱い、6.0 のボタンのピン（浮いていると毎回復旧）、上の 2・3（SLB の引数の意味、GLB に引数を入れても同じ動作か）。
- 確認済みの主張：テスト 54 件、`fcr_word` が作れる語の範囲（数まで一致）、ヘッダ CRC の範囲、検査の順序と判定、`lock_mask_at` が `logic.rs` と同一、書き込みの順序、電源断の総当たりの判定、FAT・OGFW の算術にあふれが無いこと。
  模型の中でだけ確認：書き込み中の読み取り、ロックと FLOCKE、電源断の結果の種類、4E はアプリが先の順序だけ。未確認：実際の EEFC の動作、サイズ、独立にフォーマットしたカードとの互換。
- 対応：1〜6・8・9 を Sonnet に修正させる（R60 で記録）。7 は S3 のロック前の条件として設計書 §6-3 に加える。正誤表と実機の確認項目も設計書に追記。
- 資料：データシートのテキストはスクラッチパッドの `sam3u.txt`（リポジトリには無い）。
- 信頼度：コードを読んだ結果とホストテスト（確認済み）／実機の挙動は一次資料の読解（未確認）。
- 関連：`mcu/bootcore/`、`docs/design/fw-bootloader.md`。

### R60 S2：アプリを 0x82000 にリンクする feature `resident`（Sonnet 実装、Opus が差分を確認）
- 問い：常駐部の後ろ（0x82000）にアプリを置き、S1 の封印の約束（`.app_desc`・`.image_crc`）を満たし、既定のビルドを変えずにできるか。
- 結論：できた。feature `resident` のときだけ `link/<part>_resident.x` を使う（既定の `link/*.x` は変更なし。設計書 2-5 は「その場で編集」と書いていたが、既定を変えないため別ファイルにした）。
  - リンカスクリプト：FLASH は 0x82000〜（長さ 0x1E000）、`.app_desc` を 0x82100、`.image_pad`（0xFF で埋める）＋4 バイトの `.image_crc` で 512 バイト境界に終わる。ASSERT 4 つ（ベクタ表 ≤ 0x100、記述子 32 B、トレーラ 4 B、終わりが 512 の倍数）。
  - `src/resident.rs`：`#[pre_init]` で VTOR = 0x82000（`.data`/`.bss` の初期化前。常駐部も書くが、プローブから直接起動しても動くよう二重に書く）、`global_asm!` で記述子（"NGAP"、版 1、長さと CRC は 0 で `fwimage` が埋める、ビットストリームの番地・長さ、種別と版）。
  - クロック（`embassy-atsam3/src/clocks.rs`、`resident` のときだけ）：すでに同じ設定なら何もしない。PLLA か UPLL で動いていれば、PLLA を書き換える前に MCK を主発振に切り替える。`efc::init`（FWS=4）は繰り返しても無害なのでそのまま。
  - CI：ファームのジョブに `resident`（4E）と `board-v60,resident`（2E、`DEFMT_LOG=off`）のビルドを追加（成果物は既定の ELF のまま）。
- 根拠（Sonnet の測定、`llvm-nm`/`llvm-objdump`）：既定の 4E・2E・v60 はフラッシュの内容が変更前とバイト単位で同一。4E resident：ベクタ表 0x82000、`.app_desc` 0x82100、Reset 0x82121、`.image_crc` 0x92ffc（イメージ 69,632 B）、ビットストリームは 0x100000。
  2E resident：ビットストリームはイメージ内（`.flash1` 0x92ee0）、イメージ 119,296 B（上限 122,880 B まで残り **3,584 B しかない**）。v60 resident（.rbf なし）：65,536 B、記述子のビットストリーム番地 0・種別 0。
  `fwimage seal` が3つとも成功、4E は `fwimage build --format 3` → `inspect` が更新プログラムの検査を全部通過。`mcu/bootcore/examples/check_sealed_elf.rs` が3つとも OK（1ビット反転で失敗することも確認）。
- Opus の追加：`resident` のビルドでは今のアプリ内更新（形式2、0x80000 から書く）が常駐部を消してしまうため、`fwupdate::handle` が武装を常に拒否するようにした（S5 まで）。
- 指摘された設計上の点：(1) 4E では記述子の CRC とトレーラはプレーン0（コード）だけを覆い、プレーン1のビットストリームは覆わない（LZMA の展開と FPGA 自身の CRC だけが頼り）、
  (2) `WDT_MR` は一度しか書けないので、常駐部が書いたらアプリの `wdt::disable()` は無視される。常駐部とアプリで合わせる必要がある、(3) 2E は空きが 3.5 KB しかない。
- 信頼度：ビルドと ELF・`fwimage`・bootcore の検査で確認済み。**実機では未検証**（VTOR を移した状態での embassy の割り込み、クロックの早道、常駐部からのジャンプ）。CI はまだ GitHub で走らせていない。
- 関連：`mcu/opengdemu/link/*_resident.x`・`src/resident.rs`・`src/fwupdate.rs`・`build.rs`・`Cargo.toml`、`mcu/embassy-atsam3/src/clocks.rs`、`.github/workflows/build.yaml`。

### R61 R59 の指摘の修正（Sonnet、Opus が報告と差分の範囲を確認）
- 結論：R59 の 1〜6・8・9 をすべて修正。`cargo test` 60 件通過（basics 19・fat 17・power_cut 7・update 17）。
  1. `Flash::ewp(cmd: FcrCmd, data) -> Result` の1つだけに（RAM の1関数で行う前提をコメントに明記）。模型は `ewp` の中での `read` と再入を違反として数え、電源断も `ewp` の中で起こす。
     `Timeout` を追加し、すべての EWP でタイムアウトを入れても「間違ったアプリ」は残らず、再実行で完了することをテスト。
  2・3. SLB は引数 0 だけ、GLB は引数 `0x20`（語 `0x5A00200A`、SGPB 0 との距離 2）。作れる 995 語すべてと危険な語（GPNVM 0..2 の SGPB/CGPB、EA・WP・CLB の 0..512）の距離をテスト。
     プレーン0のページ 32 以上の EWP は SGPB・CGPB・EA・CLB と距離 2 以上（距離 1 は同じページの WP だけで、書き戻しの確認で分かる）。
  4. `Action::Recover { clear_request }`、呼び出し側の手順（GPBR0 を先に消す、成功したらリセットせずアプリへ飛ぶ）をコメントで規定。
     `already_installed`：フラッシュのアプリが有効で、長さとトレーラ CRC（4E はビットストリームの CRC も）がファイルと同じなら書かない。ボタンを押したままリセット5回で EWP 0 回をテスト。真理値表は 18 行の明示的な期待値に。
  5. `FcrCmd`（非公開のフィールドにレジスタ番地と語）、`Part` を受け取り 2E のプレーン1は作れない、SLB は明示的な引数。EEFC0 = 0x400E0800、EEFC1 = 0x400E0A00（データシート）。
  6. 消去後は「記述子として読めない」ことを確認（弱いセルでも復旧できる。消去が効かなければ EWP 2 回で `Verify`）。
  8. 2回目・3回目の読み取りでイメージの数と順序を確認（読むたびに順序が入れ替わるカードで `Changed`）。
  9. `lock_mask_at` は範囲外で「全部ロック済み」を返す、Brent の数え上げは `wrapping_add`。
- サイズ：`sizeprobe` で 5,902 B（R58 の 約 5.4 KB から約 0.5 KB 増）。8 KB に入るが、HSMCI と起動コードを足すと余裕は小さい。
- **残った危険（判断した）**：SLB 0 の語は CLB 0 の語と1ビット違い。SLB の引数は 0 しか安全でないので消せない。CLB 0 になっても常駐部のロックが外れるだけで、壊れはしない。
  → 常駐部はロックの後で GLB を読み直し、外れていれば次の起動でまたロックする（D4 の「起動のたびに自己ロック」で吸収）。判断の表に記録。
- そのほかの注記：GLB に引数 0x20 を入れて同じ動作かは実機で確認（R59 の一覧のとおり）。同じイメージかどうかは長さと 32 ビット CRC で見る（衝突しても、有効で同じ長さのイメージを書き直さないだけ）。
  ボタンを押したままだと起動のたびに SD の読み取りとフラッシュの CRC 計算が1回走る（書き込みは無い）。ボタンで要求した復旧は、検証後にカードが壊れると旧アプリを失う（設計どおり、設計書に明記）。
- 信頼度：ホストテストで確認済み（Sonnet の実行。CI でも再実行される）。実機では未検証。
- 関連：`mcu/bootcore/src/{flash,fcr,boot,update,fat}.rs`・`tests/`・`sizeprobe/`。

### R62 `netgd-recv` の修正の引き継ぎ（Sonnet → Fable）と「テストが固まる」件の原因
- 問い：`sweep_medium_strided` で `test_host.exe` が固まったのは中核の無限ループか。R48 の 11 項目と CI の失敗は直ったか。
- 結論：**固まったのではなく遅かっただけ**。`run_tests.py` の作業フォルダが `test/_work_*`（E: = USB 接続の HDD、WD My Book Duo）にあり、模擬カードは書き込みコマンドのたびに flush、切断点ごとにイメージをコピーして子プロセスを 2〜3 個起動していた。
  同じテストが C:（NVMe）なら 10.3 秒、E: なら 464.5 秒（切断点あたり約 5.5 秒）。止めた時点でもファイルは更新され続けていた。
  - 中核の確認：Windows で子プロセスの上限 25 秒で通過。Linux（WSL、gcc 13）で中サイズのイメージの全切断位置（高速・低速の経路、読み戻しなし）3,270 通りを子プロセスの上限 10 秒で実行、タイムアウト 0、不変条件すべて成立。`src/*.c` のループを全部読み、上限のないものは無い。
  - 実機で上限なく待つのは KOS の g1ata の中（60 秒のウォッチドッグで対処。生の ATAPI 経路も対象に追加済み）と dcload の `/pc/` 呼び出しだけ。
  - ついでに見つかったテスト側の不具合：`sim_open` がイメージを開けないと `w+b` で空のファイルを作り、カードを黙って置き換えていた → 失敗するように修正（作るのは `mkimage` だけ）。
- テストの仕組み：子プロセスごとに上限（`NETGD_CHILD_TIMEOUT`、既定 120 秒、超えたら FAIL）、テストごとに上限（`NETGD_TEST_WORK`/`NETGD_TEST_TIMEOUT`、既定 1800 秒）、作業フォルダはシステムの一時フォルダ。
  既定は quick（351 切断点）、`--full-sweep` / `NETGD_SWEEP=full` / `CI=true` で full（567）。quick は full の 13 刻みで飛ばしていた中サイズのコミットの窓も含む。
- 結果：Windows（MSYS2 gcc 16.1）quick 29/29 OK 34.6 秒、full 29/29 OK 55.5 秒。Linux（WSL Ubuntu 24.04、gcc 13.3、CI と同じコマンド）full 29/29 OK 11.3 秒。`Makefile.host` は警告 0（`-Wextra` でも）。
- CI の修正：KOS は `-DNG_JOBDIR_DEFAULT=\"...\"` が `kos-cc` を通らなかったのが唯一のエラー → `Makefile:17` で引用符なしの `-DNG_JOBDIR_RAW=/pc/netgd/dc1` を渡し、`xport_pc.h` で文字列化（gdprobe と同じ方法。Sonnet はヘッダだけ直していた）。
  `%u` は警告だけで Sonnet が修正済み。Linux の `direct.h`・`size_t` も Sonnet が修正済み。netgd-send は 5f4e02d8 以降緑。`build.yaml` の「約 6.5 分」の注記を「約 15 秒」に修正（Opus）。
- R48 の 11 項目：すべて Sonnet が修正済みで、Fable が1つずつ確認し両方のホストで通過（UA で VERSION を1回再送、BEGIN 後のエラーで END、フォルダ引数の検査、FAT の読み取りを二重読み、FSInfo を使わない、SESSION+CHECKSUM の無いファームは書く前に拒否、
  `atapi()` の後の SLAVE 選択、64 ビットのウォッチドッグ値を割り込み禁止で読み書き、読み戻し中の取り消しと進捗、直接書き込みの前提条件の確認、一覧の切り詰めほか）。
- **残った懸念（未対処）**：4 と 5 の組み合わせで、マウントのたびに FAT 全体を1セクタずつ2回読む。128 GB・32 KiB クラスタで約 65,000 回の PIO コマンドと見積もられ、`NGD:HELLO` まで何も出力しない。数十秒〜数分かかり、PC 側のタイムアウトに当たるおそれ。
  → FAT をまとめて読んで FatFs の空き数を設定する方式が対策候補（後続の作業）。
- 未検証：KOS の実コンパイラでは未ビルド（`fwxfer_kos.c` と変更した `dev_kos.c` は次の `kos.yaml` が初めて）。スタブのヘッダで `-fsyntax-only` は警告 0。GitHub 上の Linux CI もこれから。実機では何も動かしていない。DMA の代替経路と UA の再送は模擬だけ。
- 信頼度：ホストテストで確認済み（Fable の実行）。
- 関連：`tools/netgd-recv/`（25 ファイル＋`src/fwxfer.h`・`src/fwxfer_kos.c`）、`.github/workflows/build.yaml`。

### R63 残りの課題の並列割り当て（Opus）
- ユーザーの指示「全部並列で進めて」により、実機の要らない課題を次のように割り当てた（2026-11-20）。
  - Fable：`netgd-recv` のマウント時間の改善（FAT をまとめて読み、空き数を自分で数えて FatFs に渡す。進捗の出力）。
  - Sonnet：AG16K のビットストリームを作る道筋の調査と、可能な範囲の合成（`docs/research/ag16k-toolchain.md`）。
  - Fable：2E でのビットストリームの置き場所の設計（埋め込み／SD の `NETGD.BIT`／併用／圧縮の改善、`docs/design/g1-bitstream-storage.md`）。
  - Sonnet：G3-4 の SD 読み込みの高速化（読み込み経路の調査 `docs/research/g3-read-path.md`、複数ブロック読み出し等を feature の裏で実装）。
  - Sonnet：常駐部の FAT リーダーを本物のフォーマッタ（mkfs.fat、mtools、mksdcard の出力）のイメージで試験（`src/` は変えず、問題は報告）。
  - Sonnet：GDI の引用符付きファイル名などの書式の違いへの対応。
  - Haiku：`docs/PLAN.md` を現状に合わせて更新。
- 後回し：P3 の警告掃除とクレート名の変更（`mcu/` の作業と重なるため最後）。

### R64 `docs/PLAN.md` の更新（Haiku）と、未修正だった CDI の問題
- PLAN.md を LOG R1〜R62・判断の表・git の履歴に合わせて書き直した（状況表、実装済みの一覧、ツール、フェーズ、自己更新の節。すべて実機未検証と明記）。
- **訂正**：Opus の指示では「トラックが2本の CDI でセッション2が返らない問題は R45 で修正済み」としたが、**誤り**。LOG にあるのはプリギャップの修正（R36）だけで、
  3番目以降のトラックが無いと LBA 45000 を仮定するコードは残っている（`mcu/opengdemu/src/gdi.rs:835` は CDI、`:1073` は GDI）。Haiku が LOG とコードを照合して見つけた。PLAN では未修正として載せた。
  → GDI の書式対応の作業（同じ `gdi.rs` を触る可能性がある）が終わってから修正を割り当てる。
- 信頼度：コードで確認済み（Opus が該当行を確認）。

### R65 GDI の書式の違いへの対応（Sonnet、Opus は差分の範囲を確認）
- 結論：`common/discimg/src/gdi.rs` で、`"` で始まるファイル名は閉じる `"` まで1つの名前（空白・タブを保持）、閉じない引用符は行末まで、空の `""` は空の名前、先頭の UTF-8 BOM を読み飛ばす。
  引用符なしの名前、32 バイトの上限、エラー、プリギャップの扱いは不変。CRLF/LF・タブ・行末の空白・余分な欄・大文字小文字はもともと許容しており、テストで固定。`discimg` のテストはすべて通過。
- 根拠：Flycast の `core/imgread/gdi.cpp`（引用符の中は空白を保持、エスケープなし、閉じない引用符はエラー）https://raw.githubusercontent.com/flyinghead/flycast/master/core/imgread/gdi.cpp 、
  実在の GDI の例 https://forums.libretro.com/t/dreamcast-gdi-file/17787 、列の構成 https://multimedia.cx/eggs/?p=4745 （二次資料）。
- **ファームは 8.3 の名前しか開けない**（`mcu/opengdemu/src/gdi.rs` の `Gdi::push` が `ShortFileName::create_from_str` を使い、空白や 8.3 に収まらない名前は `BadFilename`。カード上の `Track 01.bin` は `TRACK0~1.BIN` のような別名になり、GDI の文字列から導けない）。
  → 今回の修正はホストのツールにだけ効く。ファームで長い名前を扱うには、ディレクトリ走査で LFN を照合する必要がある（小さな変更ではない、後続の課題）。`netgd-send` は改名して `disc.gdi` を書き直すので、BBA 経由で書いたカードは問題ない。
- `tools/netgd-send/netgd_send/image.py`：引用符付きの名前は正規表現で対応済みだったが、BOM で `int()` が失敗していた → BOM を除去（3 行）、`TestParseGdi` を追加（7 件通過）。mksdcard は GDI を書くだけで読まない。
- 未確認：netgd-send と mksdcard の全テストはこの PC では待ちきれず、CI に任せる。
- 関連：`common/discimg/src/gdi.rs`・`tests/gdi.rs`、`tools/netgd-send/netgd_send/image.py`・`tests/test_netgd_send.py`。

### R66 6.0 のビットストリームの置き場所の設計（Fable、`docs/design/g1-bitstream-storage.md`）
- 問い：SAM3U2E（128 KB、SPI フラッシュなし）の 6.0 で、AG16K のビットストリームをどこに置くか。
- 結論（推奨）：**SD の `NETGD.BIT` から読むのを 6.0 の既定にする**。実物のビットストリームが約 45 KB 以下と分かった場合だけ、埋め込みをビルド時の選択肢として残す。
- 測定（スクラッチパッドの ELF、`DEFMT_LOG=off`）：ビットストリームの無い v60 のビルド（65,132 B）はローダーと LZMA が消えている。実際に FPGA を設定する 6.0 のアプリは約 69.1 KB。
  埋め込みの上限は、常駐部なしで 62,008 B（コードの伸びしろ 8 KB を残すと 53,816 B）、常駐部ありで 53,532 B（同 45,340 B）。見積もり 55〜75 KB は常駐部ありでは入らない。
- 圧縮の改善では埋まらない：LZMA の辞書 8 KiB で約 2 %、空の `blinky16k.bin` でも 45 % が非ゼロで RLE は逆効果。乱数のビットによる粗い模擬では 61〜84 KB（弱い根拠）。変わりうるのは AGM 独自の LZW だけ（Supra で作るまで不明）。
- 設計：SD は FPGA に依存しない（HSMCI と DMAC は MCU だけ）。ファイルは 512 バイトの OGFW 形式3ヘッダ（新しい `kind = 2`）＋生の `.rbf`。既存の更新プログラムと bootcore は未知の kind を `BadHeader` で拒否するので、誤ってフラッシュに書かれない。
  起動時に `NETGD.BIT`→`NETGD.BAK`→`NETGD.NEW` の順に探し、ヘッダを確かめて `psload` に流す。最後のチャンクはファイル全体の CRC を確認するまで送らない。新しい静的 RAM は不要（`CD_READ_BUF` を使う）。
  起動時間は埋め込みとほぼ同じ（±0.3 秒、見積もり）。失敗時は LED 5〜8（カード無し・ファイル無し・拒否・版の不一致）、MCU のファームは無傷なので壊れない（PC でファイルを入れ直せば直る）。BBA での更新は `netgd-recv` に `sysput` を追加し、NEW→BIT の入れ替えで電源断に強くする。
- 代償：どのカードにも対応する `NETGD.BIT` が必要（純正用のカードはそのままでは動かない）。カードが無いと Dreamcast からドライブが見えない。
- オーナーの判断事項：D1〜D10（設計書）。主なもの D1 SD を既定に、D2 45〜53.5 KB だった場合、D3 生の `.rbf`＋`kind = 2`、D4 最後のチャンクを保留する CRC、D7 BIT/BAK/NEW の2世代、D8 版の不一致で停止、D9 カード無しの挙動を受け入れる。
- 実機・ツールが必要：H1 Supra で作った実サイズ（基板は不要）、H2 SD 読み取り中の DCLK の一時停止、H3 途中で切れたら CONF_DONE が上がらないこと、H4 未設定の FPGA の I/O と BIOS の反応、H5 起動時間、H6 6.0 の LED・ボタン・ローダーのピン、H7 `sysput` の電源断、
  H8 純正フラッシュの `fwbackup dump`（純正はビットストリームを埋め込んでいるはず＝AGM の圧縮で小さくできる可能性）。
- 信頼度：測定はビルド済み ELF（確認済み）、時間とサイズの見積もりは推測。`bootcore` の `NAME_BITSTREAM` は未使用なのでロック前に削除する。

### R67 常駐部の FAT リーダーを本物のフォーマッタのイメージで試験（Sonnet、R59 の 7 への対応）
- 結論：`mcu/bootcore/tests/fat_real.rs` と固定イメージ 34 個（`tests/fixtures/`、非ゼロのセクタだけを保存、合計約 184 KB、生成器 `gen_fat_real.py`）。37 件すべて通過、既知の不足を表す 1 件は `#[ignore]`。
  作り方：WSL の dosfstools 4.2 `mkfs.fat -F 32`、`sfdisk`、mtools（`mcopy`・`mdel`・`mmd`）、`mksdcard --small` の出力。CI は固定ファイルだけで動く（dosfstools 不要）。
  試した形：MBR なし（クラスタ 512 B〜32 KiB）、MBR の種別 0x0C/0x0B と開始位置 2048・63・12345・8192、パーティションが項目1、32 GB 級（SD 協会の配置ほか、目的のファイルを遠いクラスタに置く）、
  BPB 0x28 の FAT のフラグ、FAT が1つ、予約セクタ最小、FAT の項目の上位4ビットに 0xF/0xA/0x5、断片化（チェーンの飛びは 6）、長い名前・紛らわしい名前・削除済みの項目・小文字（NT のフラグ）、サブディレクトリ、NetGD のカードに近い構成、4096 バイトのセクタ（拒否）。
- **見つかった不足（実在しうるカードを拒否する、`src/` は未変更）**：
  1. 有効な FAT の指定を無視して常に FAT 0 を読む。BPB 0x28 = 0x0081（ミラーなし、有効な FAT は 1）で FAT 0 が古いと `BadChain`。Windows と mkfs.fat は両方の FAT を書くので実際にはまれ。
  2. MBR の項目0しか見ない。項目0が別のパーティションで、FAT32 が項目1にあると `NotFat32`。
- Opus の判断：常駐部はロック後に直せないので、どちらも数行で直せるなら直す（判断の表に記録）。同じ Sonnet に修正させる。
- 未実施：Windows でフォーマットしたカード（管理者権限が無く VHD を作れない）→ オーナーの手作業として残す。WSL に dosfstools・mtools・parted を `apt-get` で入れた（リポジトリ外）。
- 信頼度：ホストテストで確認済み。

### R68 `netgd-recv` のマウント時間の改善（Fable）
- 結論：マウント時に FAT 1 を 256 セクタずつまとめて読み（`ngdev` の上限）、空きクラスタを自分で数えて FatFs に渡す（`free_clst`、`fsi_flag |= 1`、FSInfo の手がかりが無ければ `last_clst` も）。`FF_FS_NOFSINFO=1` は維持（数は FAT からだけ）。
  ビット化け対策：チャンクごとに2回読んで CRC-32 を比較、最大 4 組で一致しなければ `EIO`（セッションを閉じ、何も書かない）。2回読むのは、ファームの STATUS の FNV が書き込みしか保証しないため。
- 進捗：`NGD:MOUNT pct=NN`（最大で毎秒1行）、HELLO の直前に必ず `NGD:MOUNT pct=100 fat_sectors=N ms=T`（実機ではまず `ms` を見る）。PC 側（netgd-send）は HELLO を待つタイムアウトが無く、未知の `NGD:` 行は無視するので変更不要（テストで確認）。
- 根拠：模擬ディスク 4,000,000 セクタ・512 B クラスタ（FAT 30,746 セクタ）で、読み取りコマンドが **61,500 回 → 250 回**（読むセクタ数は同じ 61,500）。
  テスト 33 件（新規 4：空き数が FAT から直接数えた値と一致、1回のビット化けは再読みで回復、持続するビット化けは `EIO` でイメージ不変、大きな FAT でコマンド数と進捗行）。
  Windows quick 35 秒・full 49 秒、WSL（gcc 13.3）警告 0・quick 17 秒・full 21 秒、netgd-send 42 件 OK。
- 実機の見積もり（未測定）：128 GB・32 KiB で 31 MiB を読む。2 MB/s なら約 16 秒、1 MB/s で約 31 秒、PIO 0.5 MB/s で約 60 秒（以前は推定 1.5〜5 分）。32 GB なら約 1/4。
- オーナーの判断（実機の `ms=` を見てから）：2回読みをやめればマウント時間は半分。代わりに1ビットの化けで空き数が1クラスタずれうる（ENOSPC の事前確認は 16 MiB の余裕があり、実際の割り当ては2回読みの `get_fat` を通る）。
- 残り：256 セクタの読み取りを KOS の g1ata とファームが PIO・DMA の両方で通すか未確認（KOS の PIO が 255+1 に分ける可能性）。**`f_expand`（連続領域の検索）はまだ1セクタずつの遅い経路**で、断片化したカードでは最大約 61,000 回になりうる（後続の課題）。
- 関連：`tools/netgd-recv/src/{ngfs,ngblk,recv_core}.c`・`ngdev.h`・`ng.h`、`test/`、`README_ja.md`。

### R69 AG16K のビットストリームを作る道筋（Sonnet、`docs/research/ag16k-toolchain.md`）
- 結論：**この PC では合成もビットストリームの生成もできない**。`amaranth_env` の Yosys（0.50、wasm）は最小構成で `read_verilog`・`synth*` が無い。Windows・WSL とも yosys・nextpnr・quartus・docker は無く、`sudo` にはパスワードが要る。
  `pip install yowasp-yosys` は権限の判定で拒否されたので中止（入っても R39 の `synth_intel` の再現までで、`synth_agm` は無い）。
- オープンな道：nextpnr は AG1KLPQ48 だけ（作業中）、AG16K の配置配線は無い。`synth_agm` は Supra 同梱の Yosys の派生版にしか無い。
- Supra の道（一次・二次資料）：(1) `yosys synth_agm -vqm` → Supra `af`（rodinia の例は AG1K/AG10K、AG16K の記録は見つからず）、(2) AGM の DDF256 の手引き：Quartus で EP4CE15F23 として合成 → `.ve` でピン → Supra の Prepare/Migrate（Quartus 13.1 と 24.1 Standard で動いた報告）。
  Supra 2026.03 の win64 zip の一覧に `bin/af.exe`・`etc/AG16KDDF256.pinmap` がある（DDF256 を知っている）。ライセンスは「Trialware」の表示、再配布は不可とみられ CI には入れられない。
  配布元：AGM 公式 https://www.agm-micro.com/solutions.aspx?id=77&p=61 、archive.org のミラー（Windows 163 MB、Linux 179 MB、未取得）。フォーラムに不正なソフトの警告があり、取得するなら隔離した環境で。
- サイズ：非圧縮 552,366 B（rodinia の `blinky16k.bin`、DDF256 で同じかは未確認）、LZMA は見積もり 55〜75 KB のまま（R66 の結論は変わらない）。
- オーナーの判断：(1) WSL の venv に `yowasp-yosys` を入れてよいか（R39 の再現と Amaranth 側の確認だけ）、(2) Supra をオーナー側で取得するか（入れば基板なしで blinky → `fpga/agm/` → `synth_agm` → `af` → サイズ測定まで進められる）。

### R70 常駐部の FAT リーダーの不足2つを修正（Sonnet）と CI の結果
- 結論：`mcu/bootcore/src/fat.rs`：(1) BPB 0x28 のビット7が立ち、下位4ビットが FAT の数より小さければその FAT を読む（それ以外は FAT 0、別の FAT への再試行はしない）。
  (2) LBA0 が BPB ならそのまま、項目0が 0xEE なら `Gpt`（従来どおり）、それ以外は MBR の4項目を順に試し、種別 0x0B/0x0C・開始が 0 でない・BPB が正しい最初の項目をマウント。どれも駄目なら最初の FAT32 候補のエラー（項目0だけのカードの結果は不変）。
- 根拠：`cargo test` basics 19・fat 17・fat_real 39（ignore 0）・power_cut 7・update 17 すべて通過。新規テスト：項目0が 0xEE なら項目1が FAT32 でも `Gpt`、有効な FAT の番号が範囲外（0x85）なら FAT 0。
- サイズ：`sizeprobe` で 5,902 B → 6,104 B（+202 B）。
- CI：61dc290d で Build and Test と KallistiOS の両方が成功（**KOS の本物のコンパイラでの初めてのビルド成功**、`fwxfer_kos.c` を含む）。

### R71 トラックが2本の CDI でセッション2が返らない問題の修正（Sonnet、R64 の訂正への対応）
- 原因：`mcu/opengdemu/src/gdi.rs:831-835`（CDI）と `:1069-1072`（GDI）が `boot_area_lba` を「番号 3 以上の最初のトラック、無ければ 45000」としていた（GD-ROM の配置の前提）。
  `discimg::atapi::req_ses_reply` はセッション1＝それより前の最初のトラック、セッション2＝以降の最初のトラック。音声 LBA 0＋データ 11700 の CDI は 45000 になり、REQ_SES(2) はトラックが見つからず中断、`prefetch_ip_bin`（`ide.rs:2372`）はイメージ外の LBA 45000 を読んでいた。データ1本の CDI も 45000。
- 根拠：Flycast `core/hw/gdrom/gdromv3.cpp`（`SPI_REQ_SES` → `libGDR_GetSessionInfo`）と `core/imgread/common.h` の `Disc::GetSessionInfo`（セッション0＝セッション数、N＝`sessions[N-1]` の最初のトラックと StartFAD、GDI はセッション1＝トラック1・セッション2＝トラック3 に固定、CDI は記述子のセッションごと）。
  `docs/research/web-findings-2026-11.md` §5 と同じ結論（起動領域は最後のセッションの最初のトラック）。（一次資料、取得は抜粋）
- 修正：`common/discimg/src/cdi.rs` に `parse_info`（`CdiInfo { sessions, boot_track }`、boot_track＝最後のセッションの最初のトラック）。`atapi.rs` に `cdi_boot_area_lba` と `gdi_boot_area_lba`（GDI は従来の規則のまま、2トラックの GDI も 45000）。
  CDI のセッション1は常に最初のトラック、セッション2はセッションが2つ以上のときだけ。`req_ses_reply` の形は不変で `ide.rs` は変更なし。ファームの `cdi.rs`・`gdi.rs`、`examples/expect.rs` を合わせた。
- テスト：新規 `common/discimg/tests/cdi_sessions.rs`（データ1本、音声＋データの2セッション＝起動 11700・REQ_SES(2) はトラック2 FAD 11850、データ＋データ、3トラックの MIL-CD＝45000、1セッション3トラック、CDI の TOC、GDI の補助関数）。
  バグを固定していた `tests/atapi.rs` と `tools/netgd-expect/test_expect.py` のテストを正しい期待値に変更。`discimg` 全通過、netgd-expect 10 件（1 skip）、ファーム 4E・2E のビルド成功。
- 限界：CDI のセッションが3つ以上だと REQ_SES(2) は最後のセッションの最初のトラックを返す（起動には正しいが中間のセッションとしては違う）。`gdi.rs` の `read_ip_header` は独自の `start_lba >= 11700` の推定のまま。
  Flycast のトラックの FAD は `start_lba + pregap`、NetGD は `data_lba + 150`。特殊なプリギャップの CDI でだけ差が出る（未変更、要確認）。
- 信頼度：ホストテストとビルドで確認済み。実機では未検証。

### R72 G3-4 SD 読み込み経路の調査と高速化（Sonnet、`docs/research/g3-read-path.md`）
- 主な発見：**CMD18・DMA・4 ビット・24 MHz は既定のビルドですでに使われている**（`ide.rs:1858` `fill_cd_read_buf_raw` → `hsmci.rs:891` `read_blocks_dma`、7 セクタ（2048 B）または 5 セクタ（2352 B）ごとに CMD18＋CMD12、`sdcard.rs:167-173`）。High-Speed（CMD6）は未使用。
  エクステントの表が無いトラックは `embedded_sdmmc` 経由で 512 B ごとに CMD17、チャンクごとにファイルを開き直す（1 セクタあたり 4〜5 回、未対処）。`CD_READ_BUF` は SRAM1 の 14 KB で残り 2 KB、2 つ目のバッファは置けない。FPGA の読み出し FIFO は 4096 語。
- 時間の内訳（コードからの推定、未測定）：**DMA の CD_READ は、最後の書き込みの時点で FIFO が空でない限り最低 100 ms 待ってから完了する**（`ide.rs:2561-2571`）。ほぼ毎回、約 2 ms の転送に約 100 ms が加わる。
  SD 読み取りと FIFO への書き込みは直列で、持続 約 5.5 MB/s（2352 B なら 約 4.8 MB/s）。各要求の最初のチャンクで 1.1〜1.7 ms。
- 実装（すべて既定ではオフ、`sd-fast` で3つとも）：`fast-complete`（FIFO が空になったのを見たら 100 ms の下限を免除、ホストが読み始めなければ従来どおり待つ。リスク中：完了のタイミングは実機で壊れた前歴あり）、
  `sd-multiblock`（連続する CD_READ の後、次の範囲を空いたバッファに先読み、完了と INTRQ の後に実行。リスク低）、`sd-highspeed`（CMD6 で 48 MHz、20 ブロック×16 回を 24 MHz の基準と比較、失敗したら 24 MHz に戻る。リスク中：48 MHz での配線が未知）。
  新しいクレート `common/sdfast`（先読みの予測と CMD6 の解析、テスト 14 件）。CI の `host-tests` に追加（Opus）。
- 既定のビルドは不変：HEAD と機能オフのビルドで 2E・v60（ログなし）は SHA-256 が一致、4E（defmt あり）はサイズ同じで defmt のログ ID の並びだけ差。`sd-fast` で +1.4〜2.2 KB、RAM +56 B。`resident,sd-fast` もビルド成功。
- 未実装：SD 読み取りと FIFO 書き込みの重ね合わせ（約 +50 %、設計は文書の §8）。RAM、`cdda_task` との HSMCI/DMAC の排他、FIFO のタイミングの実測が先。
- 実機で測る順番：(1) 100 ms の下限（既定と `fast-complete` の CD_READ の所要時間、IP.BIN と 1ST_READ.BIN で起動できること）、(2) `sd-bench` を 24 MHz と `sd-highspeed` で、(3) 1ST_READ.BIN 572 セクタの読み込み時間、(4) 先読みの当たり、(5) FIFO の水位でどちらが律速か。
- 次：`fast-complete` と `enable_high_speed` は書き込む前にレビューが必要 → Fable にレビューを依頼。
- 信頼度：構造はコードで確認済み、時間は推定。実機では未検証。

### R73 G3-4 の高速化機能のレビュー（Fable、コードの読解のみ）
- 結論：先読みと `fast-complete` の条件は論理的には妥当。**`sd-highspeed` はこのまま書き込んではいけない**（検証がデータの CRC エラーを見られない）。
- 必須：
  - **M1** `hsmci.rs:940-986` `read_blocks_dma`：SR の DCRCE/DTOE（ビット 21/22）は読むと消えるのに、`xfrdone` 待ちのループ（940）と CMD12 の `cmdrdy` 待ち（961）が SR を読んで捨て、最後の `h.sr.read()`（979）では消えている。`FERRCTRL=1` で OVRE/UNRE も同様。CMD18 の応答エラー（RTOE/RCRCE）も未確認。
    新しい `read_short_block`（1116-1123）も同じ。→ `enable_high_speed`（`sdcard.rs:259-268`）の `.is_err()` はタイムアウトしか捕まえず、比較は LBA 0 の 0〜7 ブロック（MBR とほぼゼロ）だけ、8〜19 は読むだけで比較しない。
    48 MHz がぎりぎりの配線でも検証を通り、実行中にゲームのデータが黙って壊れる。**既定のビルドの読み取りでも CRC エラーを見落としている**（データシートの「読むと消える」は Fable の記憶。要確認）。
    → 各ポーリングの SR を OR で貯めて判定（既定のバイナリが変わるので別コミット）、検証は情報量の多いデータで全ブロック・より長く、HS で最初のデータエラーが出たら 24 MHz に落として再試行。
- 推奨：
  - S1 `fast-complete`（`ide.rs:2576-2591`）：FPGA の `read_fifo.level` を SMC が非同期に1回読むだけなので、2のべき乗の境目（0x400→0x3FF）で値が裂けて 0 に見えうる（推定 約 1e-5/回）。誤って 0 を見ると、Holly が読んでいる最中に DMA を止めて完了 → 短い/壊れた読み取り。→ 0 を2回続けて見る。
    （確認済み：pop はホストが DIOR を離したときなので、0 なら最後の語はホストに渡っている。水位はデバイスごと。）
  - S2 `peak` が 0 から始まる（`ide.rs:2575`）：最初の標本の前にホストが FIFO を空にすると `pulled` が立たず 100 ms と BSY を払う（既定のビルドにもある競合で、BIOS のソフトリセットの原因と注記されている）。→ `level_after_prime` で初期化。100 ms の下限は最初の取り込み 398ec7d7 由来で、FIFO を満たした後は理屈上不要。4 セクタ以下の読み取りが 1 ms 未満で完了するのは実機で未経験。
  - S3 HS の取り消しを確かめていない（`sdcard.rs:274-283`）：タイムアウトで CMD12 なしに戻るとカードはデータ状態のままで CMD6 は不正、以後の読み取りが全部失敗。→ CMD12、CMD13 で tran を確認、24 MHz で読み直し、駄目なら HS なしで初期化し直す。
  - S4 HS の検証は書き込み方向を試していない（HSMODE はホストの出力エッジを変える）。G2 の CMD25 の書き込みが 48 MHz で未検証のまま走る。→ 書き込みは 24 MHz に落とすか実機で書き込み試験。
  - S5 先読みは止められないブロッキングの SD 読み取りで、次のコマンド（ほかのコマンドやセカンダリの ATA デバイスも）を 1〜2 ms、悪ければ秒単位で遅らせ、失敗や中断した転送の後でも走る。→ `stream_from_sd` の成功時だけ、相手のデバイスのコマンド待ちも確認、CDDA の再生中は控える。
- 注記：先読みのキャッシュの正しさは問題なし（同じエグゼキュータで同期、マウント・0x52・ボタン・取り出し・WSESSION_BEGIN・ディスクの読み書きで無効化）。**既存の隙間**：ディスクへの書き込みが失敗・リセットで途中終了すると無効化が飛ばされる（`ide.rs:3506-3516`）→ 書き込みの開始時に無効化する。
  CMD6 の引数・状態のオフセット・順序（カード→HSMODE→クロック）は正しい。MCK 96 MHz で CLKDIV=0 → 48 MHz。
- 対応：警告掃除（P3、同じファイルを編集中）が終わってから修正を割り当てる。M1 は既定のビルドにも効くので優先。
- 信頼度：コードの読解（確認済み）、データシートの挙動は記憶（要確認）。

### R74 P3 警告掃除（Sonnet）
- 結論：`opengdemu` の警告 12 件（`dead_code` 11、`unreachable_code` 1）を5種類のビルド（4E・2E・v60・`resident`・`sd-fast`）すべてで 0 に。`embassy-atsam3` は元々 0。
  **フラッシュの内容は5種類ともバイト単位で同一**（`DEFMT_LOG=off`、`llvm-objcopy -O binary` の `cmp` と SHA-256。R4 の 60 バイト差より良い）。
- 方法：`main.rs` の `defmt::panic!` の後の到達しない `loop{}` を削除。残りは削除せず項目単位の `#[allow(dead_code)]`（`GdiError` の変体を消すと defmt の `Format` の番号がずれるため。`blockdev.rs` の `LbaOffset`、`gdi.rs` の `read_ip_header` ほか、`ide.rs` の `assert_intrq` ほか）。
- 残る出力：自動生成の `atsam3u4e-pac` の「lifetime elided」3,122 行（範囲外として未対処）。死んだコードの塊の削除は、defmt ありのビルドでの比較が要るので別の作業。

### R75 R73 の指摘の修正（Sonnet）：HSMCI のエラーの取りこぼし、高速化機能の安全策
- データシート（スクラッチパッドの `sam3u.txt`）：DCRCE（ビット 21、36539 行）・DTOE（22、36544 行）・BLKOVRE（24、36551 行）は「HSMCI_SR を読むと消える」。OVRE（30）・UNRE（31）は `FERRCTRL=1`（`init` が設定）で読むと消える。
  → **R73 の M1 は正しい**（既定のビルドでも DCRCE/DTOE を取りこぼしていた）。**R73 の訂正**：RINDE・RDIRE・RCRCE・RENDE・RTOE（36513-36535 行）は CMDR を書くと消える（読んでも消えない）ので、CMD18 の応答エラーは CMD12 を送る前に見れば残っている。
- 修正：
  - M1（`hsmci.rs`、既定のバイナリが変わる）：すべての SR のポーリングを OR で貯めて判定（`read_block_raw`・`read_block_dma_finish`・`read_blocks_dma_raw`・`read_short_block`・`write_block`・`MultiWrite`）。CMD18 の応答エラーを CMD12 の前に、CMD12 の RTOE も確認。
    エラー時は DMAC のチャンネルを止めて HSMCI をリセットしてから必ず CMD12（以前はチャンネルを止めず、次の読み取りが `ena1` で永久に回っていた）。BLKOVRE は誤検出を避けて対象外。呼び出し側は変更不要（GD-ROM の経路はファイルシステム経由に戻る）。
  - `read_block`・`read_blocks_dma` は `CrcError` のとき1回だけ再試行する包み（既定のバイナリが変わる。タイムアウトは再試行しない）。
  - 2（`fast-complete`）：`peak` を `level_after_prime` から、下限の免除には `pulled` と 1 ms 間隔で 0 を2回続けて見ることが必要。
  - 3・4（`sdcard.rs` の `mod hs`）：確認モードの結果が 1 であること、基準はカード上の 48 か所から選んだ情報量の多い 14 ブロックの窓を最大4つ（24 MHz で2回同じに読めるもの）、48 MHz で各窓を 48 回（約 1.3 MB）読んで全ブロックをハッシュで比較。
    取り消しは 24 MHz → CMD12 → CMD13 で tran → CMD6 で既定に戻す → HSMODE オフ → CMD6 の「変更なし」確認 → 24 MHz で読み直し、どこかで失敗したら識別モードからやり直して HS なしで初期化。実行中に `CrcError` が出たら 24 MHz に落として再読（カードは HS のまま、R73 の「HS のまま 24 MHz」）。
  - 5：HS が有効なときは書き込みの間だけ 24 MHz に下げる。
  - 6：先読みは `STREAM_OK`（全部を準備・ホストが全部取った・完了を出した）のときだけ、相手のデバイスにコマンドがあるときと CDDA の再生中は行わない。
  - 7：ディスクへの書き込みの**開始時**に CD 読み取りキャッシュを無効化（既定のバイナリが変わる）。
  - `common/sdfast` のテスト 22 件（14 → 22）。`g3-read-path.md` を更新。
- サイズ（`DEFMT_LOG=off` の text）：4E 既定 119,836（+1,028）、2E 既定 119,848（+1,028）、v60 既定 66,220（+964）、2E `resident,sd-fast` 121,108（アプリ領域 122,880 に対して残り 1.7 KB）。12 種類のビルドすべて成功、警告なし。
- **実機で最初に見ること**：M1 で今まで黙っていたエラーが出るようになる。既定のビルドで PIO の OVRE が誤って出ないか、G2 の書き込みの終わりに UNRE/OVRE が誤って出て書き込みが失敗しないか。48 MHz の検証が通るか（起動時に約 0.3 秒の見積もり）。
- コミット：M1 の部分は同じ関数の中で他の変更と入り組んでいて、対話的な `git add -p` はこの環境で使えないため、1つのコミットにした（どれが既定のバイナリを変えるかはこの項目のとおり）。
- 信頼度：データシート（一次資料）とビルド・ホストテスト（確認済み）。実機では未検証。

### R76 クレート名の変更 `opengdemu` → `netgd`（Haiku、Opus が残りの箇所を確認）
- 結論：`git mv mcu/opengdemu mcu/netgd`、パッケージ名とバイナリ名を `netgd` に（ELF は `target/thumbv7m-none-eabi/release/netgd`）。CI（成果物名 `netgd-elf`）、README・INSTALL、ツール（fwimage・netgd-expect・netgd-fwupd・netgd-recv）、docs（LOG.md は履歴なので旧名のまま）のパスを更新。CLAUDE.md と PLAN.md は Opus が更新。
- 残した旧名（意図的）：IDENTIFY の機種名 `"OpenGDEMU SD Disk"`・`"OpenGDEMU GD-ROM"`、ATA のシリアル `OPENGDEMU0000000000`、`OGDM_IMAGE_CTL`、SD の `OPENGDEMU.CFG`、FPGA のファイル名（`OpenGDEMU.rbf` ほか）、上流へのリンク、リポジトリのフォルダ名。
  `netgd-fwupd` の機種名の確認（`fwcore.c:318`）は `OPENGDEMU` と `NETGD` の両方を受け付け、テスト済み（古いファームの基板も更新できる）。
- 確認：`mcu/netgd` で 4E・2E・v60 のビルド成功、fwupdater 13 件（Windows 向け）、fwimage 33 件、netgd-recv（CI=true）、netgd-fwupd 984 項目すべて通過。Linux 向けの fwupdater は CI で確認する。

### R77 今すぐ進められるソフトの課題の並列割り当て（Opus）
- ユーザーの指示「今すぐ進められるソフトの課題を並列処理して進めて」により割り当て（2026-11-20 夜）。
  - Sonnet：ファームの長いファイル名（LFN）のトラックへの対応と、エクステントの表が無いトラックの遅い経路（R65・R72）。`mcu/netgd` のファイルシステム層。
  - Sonnet：`netgd-recv` の `f_expand` を、まとめ読みで連続した空き領域を探す方式に（R68）。
  - Sonnet：2E のフラッシュの使い道の調査と削減案（`docs/research/flash-budget-2e.md`、コードは変えない）。
  - Fable：常駐部とアプリの引き渡しの取り決め（ウォッチドッグ、クロック、GPBR、4E のビットストリームの CRC、版の番号など。R59・R60・R61・R75）。
  - Sonnet：CDI のセッション3つ以上とプリギャップの扱い（R71、`common/discimg` だけ）。
- 後回し：使われていないコードの削除（R74、ファームの作業と同じファイル）。

### R78 CDI のセッション3つ以上とプリギャップ（Sonnet、R71 の残り）
- 結論：(1) `TrackDesc` に `session: u8`（1 から、GDI は 0）を追加し、`parse_info` が記述子の順に設定。トラックにセッション番号があれば、CDI の REQ_SES(n) は n ≥ 2 でセッション n 自身の最初のトラックと FAD を返す（起動領域は従来どおり最後のセッションの最初のトラック）。
  `req_ses_reply` の形は不変。全トラックがセッション 0 なら従来の動作（ファームの `Track` はまだ `session()` を実装していないので、**ファームは従来の動作のまま**）。
  (2) **R71 の FAD の疑いは問題なし**：075b3b30 で `parse_info`（`src/cdi.rs:137-166`）が最初のデータの LBA を `start_lba + pregap - 150` と計算しているので、`data_lba + 150` = `start_lba + pregap` で Flycast の `StartFAD` と同じ（Flycast `core/imgread/cdi.cpp` の `cdi_parse`、一次資料・抜粋）。
  (3) `mcu/netgd/src/gdi.rs:894` の `read_ip_header` は**使われていない**（定義だけ）。起動は `prefetch_ip_bin`（`ide.rs:2392`）が `boot_area_lba` を使い、CDI は `cdi_boot_area_lba` 経由で正しい。
- テスト：`cdi_sessions.rs` に4件（3セッションの中間、中間セッションに複数トラック、セッション番号なしの代替動作、変わったプリギャップ 150/0/300 での LBA・REQ_SES・TOC の FAD）。`discimg` 全通過、netgd-expect 10 件（1 skip）。
- 残り：ファームの `Track` に `session` を足して `session()` を実装する（1 トラック 1 バイト、99 トラックの領域の RAM を確認してから。ファームのファイルシステムの作業の後で）。

### R79 常駐部とアプリの引き渡しの取り決め（Fable、`docs/design/resident-app-contract.md`、世代 1）
- 結論（主なもの）：
  1. **常駐部は `WDT_MR` を書かない**（一度しか書けず、リセット後は約 16 秒で動いている。データシート §14.3・§14.4.2）。復旧中は `WDT_CR`（0xA5000001）で定期的に巻き戻す。常駐部が WDDIS を書くと「永久にウォッチドッグなし」がロック領域に凍結されるため。`fw-bootloader.md` §2-3 の「復旧に入るとき WDT を止める」を覆す。
  2. 通常・復旧後・復旧失敗で旧アプリへの3経路とも、1つの `handoff()` で同じ状態にしてジャンプ（FWS を 4 に戻す、書き込み中だけ 6、クロックは `clocks.rs:102-117` の確認と同じ、PIO はリセット状態、HSMCI はリセットしてクロック停止、SysTick 停止、割り込みは一度も許可しない、FPGA のピン・PCK0・SMC・DMAC・UART・SRAM1 には触らない）。
  3. **GPBR1 を常駐部→アプリの起動報告に使うのは必須**：無いと「FPGA が失敗 → アプリが復旧を要求 → 常駐部は同じイメージなので書かない → また失敗」の無限ループ。報告の REQ ビットが立っていたらアプリは再要求しない。GPBR0 は SD に触る前に常駐部が消す（R61）。
  4. 記述子の版 1 は永久に凍結（常駐部は他の版を拒否、`bootcore/src/app.rs:74`）。新しい欄はアプリ側の拡張ブロック `"NGAX"`（+0x120、`_stext` を +0x140 に）。
  5. 4E のビットストリームの検査は常駐部に入れない。推奨 V3：プレーン1のイメージの末尾に CRC-32、アプリが拡張ブロックの期待値と比較（起動時の費用ほぼ 0）。
  6. 版：0x00081FF0 に 16 バイトの情報ブロック（`"NGRS"`、インターフェースの世代、フラグ、常駐部の版、8 KB 全体の CRC）。アプリは `52 00` の「ブートローダの版」（今は 0.0.0、`ide.rs:2336-2343`）で報告。
  7. 毎回の起動で GLB → SLB 0 は未設定のときだけ（FWS=6）→ GLB で確認、ロックに失敗しても起動は続け、結果を GPBR1 に。
- 既存コードの問題：更新プログラムのリセットが `AIRCR.SYSRESETREQ`（`mcu/fwupdater/src/main.rs:453-459`）で、周辺がリセットされるか不明 → 常駐部はリセット直後の状態を仮定しない、`RSTC_CR = 0xA5000005` に統一を推奨。
  ボタンのデバウンスで毎回 40 ms（`boot.rs:46-48`）→ 1回読んで押されているときだけ残り2回。`resident` のビルドに使えない形式2の更新プログラム（2,188 B）がまだ入っている → `cfg` で外せば 2E で約 2.2 KB 戻る（残り 1.7 KB）。
  `NAME_BITSTREAM` が `bootcore/src/fat.rs:70` に残っている（ロック前に削除）。
- オーナーの判断 E1〜E9（推奨）：E1 巻き戻しのみ／E2 `handoff()`／E3 GPBR1 と情報ブロックを世代 1 に入れる／E4 V3（優先度低）／E5 `"NGAX"` を今追加／E6 LED・ボタンのピンが分かる前に 6.0 の常駐部をロックしない／E7 `RSTC_CR` に統一／E8 LED は D6 と一緒に／E9 `resident` のビルドから形式2の更新プログラムを今外す。
- 実機の確認：H8〜H13（SYSRESETREQ 後の周辺の状態、GLB/SLB の時間、GPBR の電源投入時の値ほか）。
- 信頼度：コードは直接確認、データシートは一次資料、時間・バイト数は推定。何もビルド・実行していない。

### R80 `netgd-recv` の `f_expand` をまとめ読みの空き領域探しに（Sonnet、R68 の残りへの対応）
- 結論：マウント時の `scan_fat` で、長い空き領域の上位 64 個（開始・長さ、512 B）と、表から漏れた最長の領域を記録。`ngfs_alloc_file` は表から最も合う領域を選び、その FAT を 256 セクタずつ2回読み＋CRC で確かめて、本当に空いている連続領域を見つける。
  そのうえで `g_fs.last_clst` をその直前に設定して普通の `f_expand` を呼ぶ（R0.15b の `f_expand` は `last_clst` から探す、`ff.c:5643`）。割り当て自体は FatFs が行い、自前で FAT を書かないので重なりや空き数のずれは起きない。`FF_FS_NOFSINFO=1` は不変。
  候補が無く、スキャン後に何も解放されていなければ、FAT に触らずに EFRAG。解放があった（上書き・削除で「古い」印）なら、進捗つき（`NGD:ALLOC pct=NN`、最後に `fat_sectors`・`ms`）で1回だけ全体を読み直して再試行。
- 根拠（模擬ディスク 400,000 セクタ、512 B クラスタ、FAT 3,061 セクタ。マウント以外の読み取りコマンド数）：FAT の 85 % の位置に領域 5,408 → 242、ほぼ満杯で EFRAG 6,230 → 138。
  テスト 41 件（新規 8：ボリューム末尾ちょうどの領域、1 クラスタ足りない、検証の読み取りでのビット化けは +2 コマンドで回復、持続するビット化けは EIO、満杯のカードへの上書きは1回の読み直しで発見、`fatcheck.py` で連続性と整合性を確認）。
  Windows quick 33 秒・full 44 秒、WSL 警告 0・quick 14 秒・full 17 秒。
- 残り：表を使い切る経路（64 回超の割り当て）と代替の経路は直接テストしていない。ヒントの位置で表が間違っている（ビット化け・破損）と `f_expand` は FAT を一周しうる（`ff.c` を変えずには制限できない）。
  700 MB・32 KiB クラスタのファイルで `f_expand` と `check_contiguous` がそれぞれ約 340 コマンド。実機では 256 セクタの読み取りが通ること（R68 と同じ前提）と、`MOUNT`・`ALLOC` の `ms=` を見る。

### R81 2E のフラッシュの使い道と削減案（Sonnet、`docs/research/flash-budget-2e.md`、コードは未変更）
- 基準（aa688ba2、`DEFMT_LOG=off`、`resident`、イメージ＝bin を 512 B に丸め CRC 込み、アプリ領域 122,880 B）：2E v5.5 118,272、2E v5.5＋`sd-fast` 121,344（**残り 1,536 B**）、v60 64,512、v60＋`sd-fast` 67,584。2E v5.5 は Cyclone II のビットストリーム（LZMA 49,629 B）を埋め込み、v60 は無し（ローダー＋LZMA は約 4 KB）。
  CI は v60 resident だけを `DEFMT_LOG=off` でビルドし、2E v5.5 のビルドは無い。2E を info でビルドすると約 36 KB あふれる。CRC 表の重複なし、`core::fmt` なし、R74 の死んだコードは LTO で既に 0 B。
- 削減（2E v5.5 / v60、イメージのバイト）：(1) `main.rs:174` の起動時のデバッグ一覧 `list_root_dir(dev)` を外す −2,560 / −2,048〜−2,560（リスクなし、起動も速くなる。ログなしでもカードをマウントして2階層を歩いていた）、
  (2) `opt-level="z"` 約 −7.5 KB（速度は未測定。IDE の応答・FIFO・CDDA・LZMA に効くので実機で比べてから。`netgd` だけ `z` で熱いクレートは `s` のままなら −7,144）、(3) ASCII の `trim` の共用 −508 / −576。1+2+3 で 2E v5.5＋`sd-fast` は 110,592（**空き 12,288 B**）。
  `sd-fast` の費用：`sd-multiblock` +1,024、`sd-highspeed` +2,048。`DEFMT_LOG` は v60 で error +796・warn +7,304・info +14,816。`embedded_sdmmc` を自前の `fat.rs` に置き換えると −4〜−7 KB（見積もり、作業は大きい、LFN の作業の後で判断）。
- 埋め込みの AG16K のビットストリームの上限（LZMA）：今のまま 54,436（伸びしろ 8 KB で 46,244）、1+2 で 63,652（55,460）、`sd-fast` ありで 1+2 なら 61,092（52,900）。→ **Supra の実サイズが約 53 KB 以下なら、1+2 を入れたうえで埋め込みが現実的**。SD 方式ならフラッシュは制約にならない（R66 の結論は不変）。
- 推奨：1 を今入れる、2 は実機で速度を比べてから、`embedded_sdmmc` の置き換えは LFN の後、CI に `sam3u2e,resident,sd-fast`（`DEFMT_LOG=off`）を追加、2E の配布は `DEFMT_LOG=off`。
- 対応：1 と CI の追加は、取り決めの実装（`main.rs` を触る）が終わってから入れる。
- 信頼度：サイズはビルドで確認済み、速度は未測定。Opus の検証なし（報告とファイルを確認）。

### R82 ファームの長いファイル名（LFN）とエクステントの表に入らないトラック（Sonnet、R65・R72 への対応）
- 結論：新しいクレート `common/fatmap`（no_std、テスト 47 件）。`lfn.rs` の `LfnMatcher` は 32 バイトのディレクトリ項目を順に受け取り、長い名前を組み立てずに部分ごとに比較（RAM 約 80 B、セクタ・クラスタをまたいでよい）。
  順序番号の連続・0x40 の開始・各部分のチェックサムと 8.3 項目の一致を確認し、削除済み・孤立・不正な項目を無視。ASCII の A-Z は大文字小文字を区別しない、非 ASCII は完全一致（GDI の名前は UTF-8 として解釈、だめなら Latin-1）。
  `extent.rs` はクラスタの連鎖を連続領域に変換（プールが一杯なら続きから再開できる）、`Window` は読んでいる位置の周りの領域を必要に応じて補充。`Extent` を 12 → 8 バイトに。
- ファーム：`fat.rs` の `lookup(…, Key::Short | Key::Long, buf)` に1本化。`gdi.rs` の `PoolSink::track_file` が GDI の解析中に 8.3 でない名前を FAT32 の別名（`TRACK0~1.BIN` など）に解決して `Track::filename` に入れる → サイズの取得・エクステント・読み取りは別名を使うだけで不変。
  CDI と画像の発見は元々ディレクトリ項目から名前を取っていて対応済み、`GDEMU.INI`・`GDEMU.CFG` は固定の 8.3。
- エクステントの表に入らない原因：共有の 96 個のプールを先着順で使い、入らないファイルは丸ごと表なしになっていた。対策は feature `lazy-extents`（既定と board-v60 でオン、約 1.1 KB）：入らないトラックは窓を使って必要な分だけ連鎖を辿る。
  `embedded-sdmmc` 0.9 の `File::read` は 1 ブロックずつなので「開いたまま複数ブロック」は不可。模型では、32 KB クラスタで平均 100 クラスタの断片だと 1.2 GB のトラックで約 383 個（`g3-read-path.md` §11）。
- サイズ（`DEFMT_LOG=off` の text）：4E 既定 119,848 → 122,736（+2,888）、2E +2,888、v60 66,232 → 69,124、bss +328 B。**2E `resident,sd-fast` は 640 B あふれてリンクできない**（`lazy-extents` オフでも）。
  → アプリ側の取り決めの実装で `resident` から形式2の更新プログラム（約 2.2 KB）を外すのと、R81 の `list_root_dir` の削除（約 2.5 KB）で戻る見込み。CI は 2E `resident,sd-fast` をビルドしていないので CI は落ちないが、R81 の推奨どおり追加する。
- 残り：GDI の解析は名前を 32 バイトで切る（`discimg` の `TrackDesc::filename: String<32>`）ので、それより長い名前はまだ `Parse` で失敗する。2つの「表なし」ファイルを交互に読むと窓を作り直し続ける、後ろへのシークは連鎖を最初から辿る、窓の作り直しが CDDA のタイミングに入る。
- 実機：`Track 01.bin` 形式の名前のカードで IP.BIN と 1ST_READ が起動するか、多数のクラスタにまたがる実際の VFAT ディレクトリ、わざと断片化したトラックでの速度・後ろへのシーク・CDDA。
- コミット：`ide.rs` などをアプリ側の取り決めの作業と共有しているため、そちらが終わってからまとめてコミットする。

### R83 取り決めの常駐部側の実装（Sonnet、B1〜B9・F1〜F3）
- 結論：`mcu/bootcore`：`boot.rs` に GPBR1 の起動報告（目印 `0x4EB1`、`REPORT_BTN/REQ/WROTE/LOCKED/SLB/UNSEALED`、結果コード 0〜9、`Report::parse`、規則 X6 の `request_allowed`）、ロックの手順（`lock_step`・`lock_report`・`lock_resident`）。
  新しい `info.rs`（情報ブロック `"NGRS"` at 0x00081FF0、`IFACE_GEN = 1`、フラグ、8 KB 全体の CRC は末尾 4 B）。`app.rs` に拡張ブロック `"NGAX"`（+0x120、32 B、`min_gen`・`bs_stamp`）と `DESC_VERSION` は永久に 1 の明記。`part.rs` にクロックの定数（N3 用）。
  `Flash` トレイトに `set_fws`・`slb`（**実装側には破壊的な変更**）、`ewp_page` は FWS=6 で書いて 4 に戻す（エラー時も）。ウォッチドッグの巻き戻しの義務をトレイトの説明に。`NAME_BITSTREAM` を削除。ボタンは1回読んで押されているときだけ残り2回。
- `tools/fwimage`：FORMAT.md に「常駐部とアプリの取り決め」の節（NGAX、V3、情報ブロック、GPBR0/1、起動の手順、クロックの定数、`Flash` の義務、凍結されるもの、F5 の TODO）。`fwimage.py` に `seal-resident`、`inspect` が常駐部のバイナリを読める、`build`/`seal` が `.bs_crc` と `bs_stamp` を埋めて組み合わせを照合。
  共通ベクタ `report.txt`・`info.txt`・`ax.txt` を追加（fwimage と bootcore の両方が読む）、既存のベクタは再生成しても同一。
- テスト：bootcore 133 件（新規 34：FWS の模型＝EWP/SLB は FWS=6 のときだけ・FWS=6 のままの読み取りは違反、ウォッチドッグの巻き戻し間隔の上限＝最悪 254,008 バイト（上限 262,144、約 0.26 秒）、
  常駐部の流れの模型 20 件＝GPBR0 はカードに触る前に消す・常に復旧を求めるアプリでもリセットは1回だけ・全 EWP での電源断 約 330 通りが2回の起動で収束、ロックの手順＝SLB は初回だけ・開発用はロックしない・SLB が CLB に化けても次の起動で直る）。fwimage 48 件（2 skip）。
- サイズ：`sizeprobe` 6,104 → 6,674 B（+570 B、ハードウェアの仮の実装を含むので多めに出ている）。
- 注記：GLB が失敗すると `lock_bits` は全部 1 を返すので「ロック済み」に見え SLB は試さない（書き込みの安全のため、ただし壊れた GLB とロック済みを区別できない）。`sim.rs` は試験用で S3 の常駐部ではない。
  アプリの先頭ページを消した後にカードの読み取りが失敗すると有効なアプリが無くなり、常駐部はカードを待ち続ける（報告は書かない）。
- 信頼度：ホストテストで確認済み。実際の FWS・ウォッチドッグの動作は実機で未確認。

### R84 取り決めのアプリ側の実装（Sonnet、N1〜N9）と起動時の一覧の削除（Opus）
- 結論：`mcu/netgd/src/resident.rs`（`resident` のときだけ）：`boot_report()`（GPBR1）、`request_allowed()`、`request_recovery()`（割り込み禁止 → GPBR0 に `GPBR_RECOVER_MAGIC` → `RSTC_CR = 0xA5000005`、SYSRESETREQ は使わない）、`resident_info()`（0x81FF0）、`log_boot()`、`bitstream_stamp_ok()`。
  `ide.rs`：OGDM の版の応答のバイト 6・7 に世代とフラグ、`52 00` の先頭の語に常駐部の版（BCD）。**2E では `52 00` の版の語を省いた（約 80 B、フラッシュが足りないため、従来どおり 0.0.0）**。
  N3：クロックの定数を `bootcore::part` から取る（`embassy-atsam3` の `clocks.rs`・`efc.rs`・`lib.rs`、`netgd` の `main.rs`・`cyclone.rs`。両クレートが `bootcore` に依存）。N4：`.app_ax`（32 B、+0x120）、`_stext` を +0x140、ASSERT。
  N5：`resident` のビルドでは形式2の更新プログラムをソースレベルで外し、`build.rs` もビルドしない。N6（V3、4E）：プレーン1の末尾に `.bs_crc`、不一致なら復旧を1回要求、拒否されたら LED 10 回点滅を繰り返す（番号は仮）。N7・N8 はコメント、N9 は S5 で。
- **契約の前提の誤り**：N5 で約 2.2 KB 戻るはずだったが、元々 LLVM が `ARMED` を偽に畳んで更新プログラムは消えていて、**フラッシュは戻らない**。
- **bootcore への指摘**：`request_allowed(None)` が true を返すので、常駐部が無い（プローブで起動した）アプリが復旧を求めると同じアプリに戻り続ける → アプリ側は `report.is_some() && …` にした。bootcore 側の変更かコメントの修正を検討。
- 既定のビルド：`bootcore` への依存でクレートのハッシュが変わり関数の並びが動くが、各セクションの大きさと関数ごとの命令列は同一（4E は1関数の位置だけ）。`resident` の変更は既定のビルドでは `cfg` で消える。
- 封印：4E・2E・v60 の `resident` を `fwimage seal` → `check_sealed_elf` で OK、4E の `bs_stamp` がプレーン1の末尾 4 B と一致。
- **Opus の追加**：R81 の推奨 1 として `main.rs` の起動時の `list_root_dir` を feature `sd-list` の裏に（既定でオフ、約 2.5 KB と起動時間を節約）。長いファイル名（R82）と合わせて 2E `resident,sd-fast` がフラッシュに収まるかを確認する。
- 実機：常駐部からの引き渡しの状態（H8・H10・H11）、RSTC のリセットで GPBR0 が残るか、V3 の不一致の経路、実際の常駐部の下での `52 00` と OGDM の応答、電源投入後の GPBR の値。

### R85 R82 と R84 を合わせたビルドの確認（Opus）
- 長いファイル名（R82）・取り決めのアプリ側（R84）・`sd-list` の切り出しを合わせた作業ツリーで、9 種類すべてリンク成功（`DEFMT_LOG=off` の text）：
  4E 120,284／2E 120,296／v60 66,724／4E resident 118,656／2E resident 118,528／v60 resident 64,608／**2E resident,sd-fast 122,112（R82 では 640 B あふれていたのが収まった）**／4E sd-fast 123,560／4E sd-list 122,736。
- `netgd` の警告は 0（`sd-list` の裏に `log_entry`・`sfn_label` も入れた）。残りの約 3,130 行は自動生成の PAC。
- CI に 2E `resident,sd-fast`（`DEFMT_LOG=off`）のビルドを追加（R81 の推奨。いちばん余裕の無い組み合わせを最初に検出する）。
- 注：2E `resident,sd-fast` の text は 122,112 で、イメージ（512 B 丸め・CRC 込み）の空きはごくわずか。次に機能を足すときは R81 の `opt-level="z"` などが要る。

### R86 小さな残り3つ（Sonnet）：GDI の名前の上限、ファームのトラックのセッション番号、`request_allowed(None)`
- (1) GDI の名前の上限を 32 → **128 バイト**（`discimg::MAX_NAME`）。255 にしなかったのは、ファームが長い名前の照合中に名前を約5回持つため（128 で約 650 B、255 だと約 1.3 KB のスタック。2E の SRAM0 は約 8 KB 空き）。
  超えたら新しい `discimg::Error::NameTooLong`（ファームでは `BadFilename`）。`Track::filename` は 8.3 の別名のままで、99 トラックの領域 `TRACKS` は 0xC64 B で不変。Redump 形式の長い名前のテスト、128/129 の境界、255 文字の VFAT 名の照合を追加。
- (2) ファームの `Track` に `session: u8`（構造体の末尾の詰め物に入り RAM 増加 0）と `TrackInfo::session()` → セッション3つ以上の CDI の REQ_SES(n) がファームでも有効（R78 の残り）。
- (3) `bootcore::boot::request_allowed(None)` を **false** に（常駐部が無いと復旧の要求でループするため。R84 の指摘）。FORMAT.md の X6、`resident-app-contract.md` §3-2、`resident.rs` を更新。
- 確認：discimg・fatmap・bootcore・fwimage（2 skip）・netgd-expect（1 skip）すべて通過。ファーム text：4E 120,360、2E 120,372、v60 66,796、**2E resident,sd-fast 122,120（512 B 丸めと CRC を入れて残り約 244 B）**。
- 注：`link/*.x` の「スタックは約 2 KB」のコメントが古い（実際は約 8 KB 空き）。

### R87 成果物の Google Drive への配置（Opus）
- ユーザーの指示「成果物は Google Drive にも UP して」により、コミット 35285878（CI の Build and Test・KallistiOS とも成功）の成果物を、ユーザーの Drive の新しいフォルダ `NetGD` に置いた（非公開、共有設定は変えていない）。
  - `firmware/`：`netgd-v55-4e`・`netgd-v55-2e`・`netgd-v60-2e-nobitstream` の `.bin` と `.elf`（`DEFMT_LOG=off` のリリースビルド、このPCでビルド）。resident 版は常駐部本体が未完成のため入れていない。
  - `dreamcast/`：`netgd-recv.elf`・`netgd-fwupd.elf`・`gdprobe.elf`（KOS の CI 実行 44717854323 の成果物 `dreamcast-elfs`）。
  - `docs/`：README.md・PLAN.md・LOG.md・user-tasks.html。最上位に `はじめに読む.txt`（各ファイルの説明、実機未検証、書き込む前に純正ファームをバックアップすること）。
- Drive の 4E の .bin は Drive の判定で種類が別の表示になったが、内容（573,920 B）はそのまま。
- 以後の更新：Drive は自動では同期しない。新しい版を置くときは同じフォルダに上書き（またはコミット名の付いたサブフォルダ）にする。

### R88 実機なしでできる残りの作業（Opus 統括。Fable：`read_ip_header` と `trim`、Sonnet：コメント、Opus：PLAN）
- 問い：実機・Supra を待たずに今できるソフトの課題は何か、それを片づける。
- (1) **PLAN.md を最新に**（Opus）：D-1 は R71・R78・R86 で修正済み、マウント時間は R68・R80 で対策済み、P3 は R74 で完了、G3-4 は `sd-fast` で実装済み、CI は 35285878 で全成功（R87）、と書き換え。「進行中」を「ソフト側の残り」に置き換えた。
- (2) **`read_ip_header` を起動領域の判定にそろえた**（Fable）：独自の「`start_lba >= 11700` を先に」の推定をやめ、新しい純粋関数 `discimg::atapi::ip_bin_candidates(tracks, boot_area_lba)`（起動領域以上のデータトラック → それ未満のデータトラック、音声は除く）を使う。
  旧と結果が変わるのは「セッション3つの CDI（データ 11700 / 28250）」と「データ＋データで2番目のセッションが 11500」の2例だけで、どちらも新しい第1候補は `prefetch_ip_bin` が読む LBA と一致。GDI は実在の盤では不変。
  **`read_ip_header` は呼び出し元が無い（`#[allow(dead_code)]`、LTO で消える）**ので、ファームの挙動とサイズは不変。`prefetch_ip_bin`（`ide.rs`）は元から `gdi.boot_area_lba` を使っている。テスト `cdi_sessions.rs` に5件（旧規則を補助関数として残し、一致と差を固定）。
- (3) **`trim` の共用**（Fable、`flash-budget-2e.md` の案 3）：`gamecfg::trim`（`#[inline(never)]`、空白と 0x09〜0x0D だけ）を `gamecfg` の全 `.trim()` と `gdi.rs:623` で使う。テスト3件。
  text：4E −200、2E −500、2E `resident,sd-fast` 122,120 → **121,624**（−496）、v60 `resident` −496。**挙動の差：全角スペース（U+3000）や U+00A0 を削らなくなる**。日本語で編集した `GAME.CFG`・`GDEMU.INI` の行の前後にあるとキーが一致しない（実機で1回確認したい）。
- (4) **古いコメント**（Sonnet、コメントのみ）：`ide_device.py` に `m.Case` と照合したスロット表（0〜31）を追加（「32〜127 は 0」は `mcu_out` の既定値に依存、未確認）。`ide.rs` の FIFO の深さのコメントを主 4096 語・副 1024 語に統一。
  `link/*.x` に実際のスタックの余裕を追記（`_stack_start - __sheap`、`llvm-nm`：2E 約 7.4 KB、4E 約 19 KB）。「約 2 KB のスタック」は実はリンカスクリプトではなく `cdi.rs:64`・`gdi.rs`・`systick.rs:18`・`ide.rs:2070`・`sdcard.rs` のコメントにあり、未修正（範囲外）。
  - 警告：`netgd`・`embassy-atsam3` の警告は3種類のビルドとも 0（PAC 以外）。R74 のとおりで直すものなし。
  - バイト同一性：`DEFMT_LOG=off` の2種類は同一。**既定（defmt あり）の 4E は `ide.rs` のコメント変更だけで 6 バイト変わった**（defmt の文字列番号の入れ替わりとみられる。推測、未確認）。動作には影響しないので受け入れた。
- 確認（Opus、合わせた状態）：`common/discimg`・`common/gamecfg` の `cargo test` 全通過、`cargo build --release`（4E）と `DEFMT_LOG=off --features sam3u2e,resident,sd-fast` 成功（text 121,624 / data 156）。Fable の報告：netgd-expect 10 件（1 skip）、v60 `resident` もリンク。Sonnet の報告：bootcore 全通過、`main.py` の再生成で `out/` 不変。
- 信頼度：ビルド・テストは確認済み。実機は未検証。Opus はコードの差分を確認（`gdi.rs`・`atapi.rs`）。
- 関連：`common/discimg/src/atapi.rs`、`common/discimg/tests/cdi_sessions.rs`、`common/gamecfg/src/{lib,ini}.rs`、`mcu/netgd/src/{gdi,ide}.rs`、`mcu/netgd/link/*.x`、`fpga/amaranth/hw/ide_device.py`、`docs/PLAN.md`。

### R89 Fable による全体のデバッグ（Fable×3、読むだけ。詳細は `docs/research/fable-debug-2026-11-20.md`）
- 問い：ユーザーの指示「fable で全体をデバッグ」。HEAD 84a4d180 で、実機なしに見つかるバグを領域別に探す（A ファームの読み込み経路、B 自己更新と起動、C G2 の書き込み経路）。リポジトリは変更なし。
- 結論：**基板を壊す種類のバグ（B）と、受信の中核のデータ破損（C）は見つからなかった。** 直すべき確認済みのものは次のとおり（重い順、詳細と直し方は別ファイル）。
  - A1 同じデバイスへの新しいコマンドで、複数チャンクの CD_READ・ディスク読みが止まらず、新しいコマンドの CDB・バイト数を上書きして 5〜10 秒遅れる（高〜中、コードと RTL から）。
  - C1 送り終わった `put` が `job.ngj` に残り、次の送信で前のイメージを消して送り直す（中〜高、使い捨てテストで確認）。
  - B1 `fwimage build --target 4e` が 2E の ELF を通す。今の 4E 基板に書くと SWD でしか戻せない（形式2で高）。
  - A2 DMA の CD_READ が途中の SD 失敗でも成功を返す。C3 v2・v3 の CDI を送れるがファームは v3.5 しか読めない。A3 日本語などの 8.3 別名の長い名前のトラックをマウントできない（中）。
  - A5 GDI のセクタサイズ 0 で 0 除算のパニック（既定のイメージなら毎回の起動で）。A4 起動時のマウント失敗で `MEDIA_PRESENT` が true のまま。B2 `sd_recover_ex` が FWS を入れずにプレーン1を読む。C2 古い `force.req` が残り保護が無効のまま。C4 ドライブをまたぐ `/pc/` のパスのずれ。A6 `sd-bench`・`fpga-diag` がコンパイルできない。A7 0xA0・0x08 の役割の確認なし。C7 壊れた FAT の値で LBA が桁あふれ。ほか低いもの多数。
  - 常駐部をロックする前に決めること：B5 EWP のタイムアウトで戻らない、B6 復旧の途中のリセットで REQ のループ防止が消える（GPBR0 の意味）、B7 WDT の巻き戻しと WDT_MR の間隔、B8 書き直しの回数の上限。
  - 実機が要る疑い：A-a FIFO の量の非同期読み（繰り上がりの途中）、A-c コマンドの取りこぼし、A-d HSMCI（CMD12 の間のエラー、SDSC）、A-e CDI の CDDA とデータの交互読みで FAT のたどり直し、C-A 256 セクタの PIO 書き込みの分割。
- 根拠：各担当の報告（ファイル:行、使い捨てテストはスクラッチパッド）。テスト：bootcore 133・fwupdater 13・fwimage 48（2 skip）・netgd-recv quick 41・netgd-send 43・discimg・fatmap・sdfast・gamecfg すべて通過。ファジングで GDI 30 万・CDI 20 万・LFN 30 万件、パニックなし。
  2E `resident,sd-fast` のスタック：使える 7,160 B、静的な最深 約 6.2 KB（`load_by_name` 3,360 B が最大）、割り込み用の残り約 1 KB。
- Opus の検証：A2（`ide.rs:2547`・`:2677`）、A4（`ide.rs:986`）、A5（`gdi.rs:890`）、B2（`update.rs` の `set_fws` は `ewp_page` の中だけ）、C1（`job.py:233-242`）、C3（`cdi.rs:81-86` と `image.py:227-235`）をコードで確認。その他は担当の報告のまま（未検証）。
- 信頼度：確認済み（ビルド・テスト・コード）と推測が混在、表ごとに区別。実機は未検証。
- 関連：`docs/research/fable-debug-2026-11-20.md`。修正はまだしていない。

### R90 R89 の指摘の修正（ユーザー「全部」。Fable×2・Sonnet×2 を worktree で並列、Opus が統合）
- 問い：R89（`fable-debug-2026-11-20.md`）の確認済みの指摘をすべて直す。担当ごとにファイルを分け、各自の worktree のブランチで作業→Opus が main にマージ（衝突なし）。
- **ファーム `ide.rs`（Fable、573e95e5）**：
  - A1 `wait_host` に自デバイスの新コマンドで抜ける `Stall::OwnCommand`。`aborted()`（リセット、または opcode ≠ 0 の pend）と `disarm()`（スロット 19 に 0）を足し、`stream_from_sd`・`stream_pio_from_sd`・`handle_disk_read`・`handle_disk_write`・`send_data_in` で状態・バイト数・IRR を書く前に確かめる。中断時は何も返さず次のコマンドへ。
  - A2 DMA の CD_READ の途中の SD 失敗はエラー。**SD の失敗は両方の CD_READ で Error 0x34（MEDIUM ERROR、sense key 3＋ABRT）に**（従来 0x04。ホストの停止による失敗は 0x04 のまま）。
  - A4 `ide_task` の開始時に `MEDIA_PRESENT = gdi_ref().is_some()`（起動時にイメージが無ければ NOT READY、ディスクへの書き込みは受け付ける）。A7 0x08・0xA0 は `Role::Packet` だけ。
  - A8 書き込み FIFO の量が足りなければ CMD25 に流さず失敗／長さ 0 のデータ段階は DRQ なしで完了1回／`size_disk_from_mbr` に BPB の確認／既定のマウントの失敗（SUB_MOUNT の NO_SELECTION、`52 82 00 00`）は ILLEGAL REQUEST（GDmenu・openMenu には新しい挙動）。
  - A-a FIFO の量を2回読み（書き込みは小さい方、`room()` は大きい方）。A-b リセット・新コマンドの後に状態・INTRQ を書かない。A-c `ide_task` は両デバイスに pend が無くなるまで見直す。**`ack_command` の取りこぼしは未修正**（スロット 0 が「1 を書く」レジスタで、FPGA の変更が要る）。
- **ファームのそれ以外・discimg・fatmap（Sonnet、805f8c6f）**：A3 `resolve_long` は項目の生の 11 バイトを返す（`short_from_raw`、embedded-sdmmc 0.9.0 の `ShortFileName { contents: [u8; 11] }` への `transmute_copy`、サイズと整列をコンパイル時に確認。**クレートの版を上げるときは要確認**）。非 ASCII の名前は長い名前の照合を先に。
  A5 セクタサイズは 2048/2336/2352 だけ（discimg とファーム）、0 除算なし。A6 `sd-bench`・`fpga-diag` がコンパイル可。A8 `MAX_GDI_BYTES` 4 KB → 12 KB（既存の 14 KB の `loader_scratch` から、RAM 増加なし）、溢れたら Parse／`lookup` の循環の上限／非 ASCII のフォルダ名の照合 `widened_eq`。
  C3 CDI の版：v3.5 は末尾から、v3（0x80000005）は先頭から、v2 などは BadCdi（根拠は Flycast `cdipsr.cpp`、二次資料、実物の v3 では未確認）。C6 GDI の数値の範囲検査と先頭の空行。C7 `fat_next` はクラスタ数で範囲検査、`cluster_lba` は `Option`、fatmap の `checked_cluster_lba` と `Walk::Broken`。
- **自己更新（Fable、4dc49750）**：B1 `fwimage build` は SP が対象の SRAM0 の終わりと一致しなければ `BadTarget`（`--any-sp`）、プレーン1の無い 4E は `NoBitstream`（`--code-only`）。B2 `sd_recover*` の最初に `set_fws(FWS_RUN)`、模型は FWS 未設定から。B3 プレーン1の最後の語も比較（`bitstream_is`）。B4 参照の修正。
  B5 EWP のタイムアウトは戻らない（RAM のルーチンが `RSTC_CR`）。B6 GPBR0 の第2の値 `0xBCADB8B1`（復旧中）、見つけたら復旧せず REQ＋`RESULT_INTERRUPTED`(10)。B7 引き渡しで WDT_CR の巻き戻しを最初に（ジャンプの 100 µs 以上前）。B8 書いた復旧の失敗は1回の起動で3回まで（`Fallback::GiveUp`）。
  B9 sizeprobe は FCR の前に毎回 FSR、`lock_bits` は FCMDE で None（sizeprobe 6,674 → 7,034 B、8 KB 以内）。B10 fwupdater の `resident`：ビット0を見ない、ページ 0〜31 の EWP なし、`RSTC_CR` でリセット（形式2は不変、2,188 → 2,196 B）。B11 FORMAT.md に 4E 形式2の2イメージの窓。B12 GLB の失敗は `LOCK_UNKNOWN`（GPBR1 ビット14）、アプリが有効なら今回の復旧は `LockUnknown`(6)。
  取り決めの変更は `resident-app-contract.md` §10 の表。**`IFACE_GEN`・`DESC_VERSION` は 1 のまま**（常駐部もロックされた基板もまだ無く、区別すべき旧世代 1 が存在しないため）。
- **G2 ツール（Sonnet、f51b58a9）**：C1 前回の `result.ngr` で `DONE ok` の put を `job.ngj` から外す（警告つき）。C2 古い `reload/force/fragok.req` を `prepare` で消す、`force.req` は止められなかった実行の後にアプリが消す。C3 `check_cdi` は v3・v3.5 だけ。C4 ドライブが違えば `JobError`（`--placement copy`）。C5 .gdi は `
` で分け UTF-8→ANSI→latin-1。
  C6 サイズの上限 0xFFFFFFFE と `ngfs.c` の 64 ビット計算。C8 空のファイルの直後での再開。C10 `f_expand` の失敗で領域を表に戻す。C11 GDEMU.CFG を最大 64 KiB まで全部読む。C12 名前のバッファ 256、入らなければ NG_E_IO。C13 ヒントなしの `f_expand` を呼ばず EFRAG。C-A 書き込みは1コマンド 128 セクタまで。
- 確認（Opus、4つを合わせた main）：host テストは discimg・fatmap・gamecfg・sdfast・psload・lzma・bootcore（143）・fwupdater（15、`resident` も 15）・fwimage（55、3 skip）・netgd-recv（47）・netgd-send（52）・netgd-expect（10、1 skip）・gdprobe すべて通過。
  ファームのビルドはすべて成功（text+data）：4E 162,548+144、4E `resident` 161,900+332、2E 122,244+156、v60 68,696+156、v60 `resident` 66,568+408、**2E `resident,sd-fast` 122,112+156 = 122,268（上限 122,368 まで残り 100 B）**。`cargo check` で `sd-fast,fpga-diag`・`sd-bench`・`sd-list` もエラー 0。
- 注意：2E `resident,sd-fast` の残りは 100 B。次に機能を足すときは `flash-budget-2e.md` の案 2（`opt-level="z"`）か `handle`・`handle_packet` の `#[inline(never)]` を先に。`ide.rs` の修正は関数のインライン化の偶然で増減が隠れている（実費 約 +400 B）。
- 信頼度：ビルドとホストテストは確認済み。実機は未検証。**実機で見ること**：転送中の新しいコマンドで止まる（A1）、CD_READ の失敗時の 0x34 への BIOS の反応、起動時にイメージが無いときの NOT READY、既定のマウントの失敗での GDmenu・openMenu の反応、副への 0x08 の ABRT。
- Opus の検証：各ブランチの差分の範囲とマージ結果、全テスト・全ビルドを確認。`short_from_raw` の前提（embedded-sdmmc 0.9.0 の構造体）を確認。個々のロジックは担当の報告のまま。
- 関連：`mcu/netgd/src/{ide,fat,gdi,sdcard,main,resident}.rs`、`common/{discimg,fatmap}`、`mcu/{bootcore,fwupdater}`、`tools/{fwimage,netgd-send,netgd-recv}`、`docs/design/resident-app-contract.md`、`tools/fwimage/FORMAT.md`。

### R91 Google Drive の成果物を R90 の版に更新（Opus）
- ユーザーの問い「成果物は Google Drive に up した？」→ R87 の版（35285878）のままだったので更新。Drive の `NetGD` フォルダで、旧版の `firmware/`・`dreamcast/`・`docs/`・`はじめに読む.txt` を新しいフォルダ `old-35285878/` に移し（削除はしていない）、同じ構成で新しい版を置いた。
  - `firmware/`：e9df6141 をこの PC で `DEFMT_LOG=off` のリリースビルド。`netgd-v55-4e`（bin 573,920 B）、`netgd-v55-2e`（bin 122,400 B）、`netgd-v60-2e-nobitstream`（bin 68,852 B）の `.bin` と `.elf`。resident 版は常駐部本体が未完成のため入れていない（R87 と同じ）。
  - `dreamcast/`：KOS の CI 実行 44751974784（e9df6141、成功）の成果物 `dreamcast-elfs`（`netgd-recv.elf`・`netgd-fwupd.elf`・`gdprobe.elf`）。
  - `docs/`：README.md・PLAN.md・LOG.md（R91 まで）・user-tasks.html・fable-debug-2026-11-20.md。最上位に新しい `はじめに読む.txt`（R90 で変わった挙動を追記）。
- 確認：Drive が返したファイルサイズがローカルと一致。2E の .bin は Drive の判定で種類が別の表示になる（R87 と同じ、内容はそのまま）。
- 以後：新しい版を置くときは同じやり方（旧版を `old-<コミット>/` に移す）。

### R92 Supra 2026.03（Windows 版）の静的解析（Opus、詳細は `docs/research/supra-static-analysis.md`）
- 問い：出所のはっきりしない Supra の実行ファイルを、動かさずにどこまで確かめられるか。ユーザーがダウンロードを許可（「ダウンロード許可します。解析して」）。VirusTotal へのハッシュの照会は返事が無かったので、していない。
- 結論：**静的に見る限り、マルウェアらしい兆候は無い。** ただし AGM 製の主な実行ファイル（`Supra.exe`・`af.exe`・`Downloader.exe`・`yosys.exe`）は署名が無く、取得したのは Internet Archive の写しで公式の配布物との一致は未確認。「バックドアが無い」とは言えない。
- 根拠：
  - zip は 171,183,087 B、SHA-256 `81819687…5e2c4`、MD5・SHA-1 は Internet Archive のメタデータと一致。危ないパスなし、CRC 正常。
  - インポート表：socket・NETAPI32 などは同梱の Tcl と同じ組み合わせ（`yosys.exe` も同じ）。`GetAdaptersInfo` はノードロック型ライセンスの MAC とみられる。`CheckRemoteDebuggerPresent` あり。注入の API（`WriteProcessMemory` など）は無し。
  - 文字列：埋め込みの通信先 URL・IP は無し（Qt・XML・論文・CRL だけ、ドメインの一覧は Qt の公開サフィックスの表）。パッカーの跡なし（エントロピー 6.0〜6.4）。`.bat` は PATH を通して呼ぶだけ。
  - ESET `ecls.exe /clean-mode=none`：ファイル 1,827・オブジェクト 72,736、検出 0（定義 20261120）。Defender はこの PC では停止中。
- 注意：`urjtag/inpout32.dll` は `CreateService` でカーネルドライバ（`inpoutx64.sys`）を入れる（悪用の知られた古いドライバ）。urjtag は使わない。`zadig` は USB ドライバを差し替える。正式版のライセンスを要求すると、PC の識別情報が AGM に渡るとみられる。
- **R69 の訂正**：Supra 2026.03 の Yosys 0.61 に **`synth_agm` は無い**（上流の `synth_*` だけ）。AGM 向けの写像は `etc/af_map.tcl`（`yosys -import`）と `etc/arch/alta-*.ar`。`etc/AG16KDDF256.pinmap` はある。
- 勧め：動かすならネットワークを切った VM か別の Windows で、`af.exe`・`yosys.exe` だけを使う。次は小さな設計を通して通信が出ないかを見てから、NetGD の AG16K 版でビットストリームの実サイズを測る（実行はユーザーの判断）。
- 信頼度：ハッシュ・署名・インポート・文字列・ESET の結果は確認済み。用途の解釈（Tcl・ライセンス・Qt）は推測。Opus 単独で、Fable の検証なし。ファイルはスクラッチパッドにあり、リポジトリには入れていない（再配布にあたりうるため）。

### R93 Supra 2026.03 のハッシュを VirusTotal に照会（Opus、R92 の続き）
- ユーザーの許可「VirusTotal に照会してよい」により、内蔵ブラウザで `virustotal.com/gui/file/<SHA-256>` を開いた。送ったのは SHA-256 だけで、ファイル本体は送っていない（アップロードはしていない）。
- 結果：
  - **未登録（Item not found）**：zip（`81819687…5e2c4`）、`af.exe`、`Supra.exe`、`Downloader.exe`、`yosys.exe`、`openocd.exe`、`agrv32flash.exe`。**AGM 製の主な実行ファイルは、誰も VirusTotal に出したことが無い**ので、VirusTotal からは判断材料が得られない。
  - 登録あり・検出 0：`python_dist/python.exe`（約 70 エンジン中 0、照会の仕組みが働いていることの確認）、`urjtag/inpout32.dll`（71 中 0、「File distributed by Tenasoft SRL」）、`urjtag/jtag.exe`（約 70 中 0、2年前の解析）。
- 結論：R92 の判断は変わらない（静的にはマルウェアの兆候なし、言い切れない）。AGM 製のファイルの評判は VirusTotal に無い。ファイルをアップロードすれば約 70 のエンジンで調べられるが、ファイルが公開され VirusTotal の提携先に共有される（AGM のソフトの再配布にもあたりうる）ので、しないままにした（ユーザーの判断事項）。
- 信頼度：照会結果は画面で確認済み（分母の数は画面の読み取り）。

---

### R94 Supra の AGM 製実行ファイルを VirusTotal にアップロードする試み（Opus、R93 の続き）

- 担当：Opus。ユーザーの許可：「すべて上げる」（`af.exe`・`Supra.exe`・`Downloader.exe`・`yosys.exe` の4つ）。
- 問い：AGM 製の4つを VirusTotal に上げて、約 70 エンジンの判定を得られるか。
- 結論：**アップロードはできなかった。何も送っていない**。
  - 組み込みブラウザにはファイルを上げる機能が無い。Claude in Chrome の `file_upload` は1回 10 MB までで、指定した `<input type=file>` の ref が要る。
  - 1回 10 MB を超えるので、zip にした（Compress-Archive）：`af` 5,295,481 B、`Supra` 9,493,900 B、`Downloader` 9,513,972 B。`yosys` は zip で 19,477,052 B、7z（`-mx=9`、BCJ2）でも 11,418,336 B で、10 MB に入らない。
  - VirusTotal のアップロード画面は shadow DOM の中にファイル欄があり、`find` と `read_page` からは見えない（隠しの textarea しか取れない）。`/old-browsers/` も同じ画面に戻される。3回試して止めた。
  - API（`/api/v3/files`）は API キーが要るので、Claude からは使わない（キーの入力は禁止事項）。
- 根拠：4ファイルの SHA-256 が R92 の値と一致（`sha256sum`）。ブラウザでの操作結果は上のとおり。
- 信頼度：確認済み（アップロードしていないこと）。
- 次の段階：ユーザーが自分で https://www.virustotal.com/gui/home/upload に `scratchpad/supra/x/bin/` の3つと `map/bin/yosys.exe` をそのまま上げる（Web 画面は 650 MB まで受け付ける）。結果は新しい項目として記録する。
- Opus による検証：該当なし（Opus 自身の作業）。
- 関連：`docs/research/supra-static-analysis.md`、R92、R93。

### R95 Supra 2026.03 の追加の検証：他の脅威情報、深い静的解析、出所（Opus・Fable、R94 の続き）

- 担当：
  - Opus：ハッシュ照会と出所の確認。
  - Fable：深い静的解析（サブエージェント）。Opus が要点を逆アセンブルで確認した。
- ユーザーの指示：「他の方法で検証して」。
- 問い：VirusTotal に上げられないなら、ほかの方法で安全性をどこまで確かめられるか。
- 結論：
  - **Kaspersky OpenTIP** に `af.exe`・`Supra.exe`・`yosys.exe` の記録があった。3つとも、2026-05 から約 10 回観測されていて、未分類で検出名は無い。MD5 も一致した。`Downloader.exe` と zip は登録が無い。
  - **`Supra.exe` のフックは WH_GETMESSAGE（3）で、Qt のもの**だった。キー入力を記録する種類ではない。
  - 情報を盗む・遠隔操作する・居座る・解析を妨げる仕組みは見つからなかった。ネットワークのコードは Tcl の `socket` の中だけ。埋め込みのファイルやオーバーレイも無い。
  - 4つは同じ環境で1時間以内に続けてビルドした形になっている。
  - 公式の配布物と同じかどうかは、まだ確かめられていない（Web に公式のハッシュが無い）。
- 根拠：
  - `docs/research/supra-static-analysis.md` §9。
  - OpenTIP の結果ページ：`opentip.kaspersky.com/<sha256>/results`。
  - `objdump -d --start-address=0x1408858b0` の出力：`mov $0x3,%ecx` → `call *0xbfd1e2(%rip) # 0x141482ac8`（SetWindowsHookExW の IAT）。
  - `md5sum` の結果。
- 信頼度：
  - 確認済み：ハッシュ照会、フックの種類、import の参照元。
  - 推測：TLS コールバックのうち1つが jemalloc のもの、`GetAdaptersInfo` の用途、`Supra.exe` の 16 KB の高エントロピー領域の中身。
- Opus による検証：あり（フックの呼び出し箇所を逆アセンブルし、MD5 を照合した）。
- 関連：R92、R93、R94、`docs/research/supra-static-analysis.md`。

### R96 Drive の成果物を c821d0c5 に更新（Opus）

- 担当：Opus。ユーザーの指示：「検証はこれで良しとします。GoogleDriveにupして」。
- 問い：e9df6141（R91）から変わった成果物を Drive に反映する。
- 結論：
  - e9df6141 から変わったのは `docs/research/LOG.md` と `docs/research/supra-static-analysis.md`（新規）だけで、ファームウェアなどは変わっていない（`git log e9df6141..HEAD --stat`）。
  - Drive の `docs/` に、新しい `LOG.md`（218,057 B、R95 まで）と `supra-static-analysis.md`（16,102 B）を上げた。
  - 古い `LOG.md` は新しく作ったフォルダ `old-e9df6141/` に移した。
  - Supra の実行ファイルや zip は上げていない（再配布にあたるため）。
- 根拠：Drive が返した大きさが手元の `wc -c` と一致した。フォルダは https://drive.google.com/drive/folders/1u4Rmb1VWBFqc2f2mUeHfYpwxwod3c65H 。
- 信頼度：確認済み。
- Opus による検証：該当なし（Opus 自身の作業）。
- 関連：R91、R95。

### R97 NetGD 専用メニュー `netgd-menu` の設計（Fable、Opus が一部を確認）

- 担当：Fable（設計）。ユーザーの指示：「すべてやる」「役割を与えて並列処理して」。
- 問い：GDmenu・openMenu の代わりになり、BBA 経由の書き込み（G2）をまとめた Dreamcast 側のメニューを、どう作るか。
- 結論：詳細は `docs/design/menu-app.md`（9e063d7b、main に取り込み 2596acef）。
  - フォルダ `01` に置く KOS アプリにする。メニュー自身が SD を読み取り専用で走査して一覧を作る。
  - 起動は、`52 82 lo hi` で番号を指定してマウントし、READY を待ってから BIOS に渡す（既定は `arch_menu()`）。
  - **ファームの変更は要らない**。
  - BBA があれば、取り込み画面で PC から SD に書ける。BBA が無くても、一覧と起動は動く。
  - 取り込みのプロトコルは新しい T2（単体の TCP）。Dreamcast が 64 KiB ずつ要求し、PC が答える。結果は既存の `NGD:` 行で返す。G2 設計 5.4 の表は、この設計の §8 で置き換える。
  - netgd-recv の中核（`recv_core.c`）は変えずに、相対参照で使う。追加するのは、読み取り専用のフラグ、FAT 全体を走査しない軽いマウント、`fwctl`。
  - 実装は 12 の手順。手順 1〜4 は KOS が要らない。手順 9〜12（UDP 探索・ファーム更新・箱絵・実機の数字で決めるもの）は任意。
- 根拠：
  - 設計書の §5.2（コマンドのバイト列）と §8（プロトコル）。
  - CI が `tools/*/Makefile` のうち `Makefile.rules` を含むものを全部ビルドすることは、Opus が `tools/ci/kos-build.sh` の 31〜40 行で確認した。
- 信頼度：
  - 設計：推測（実機は未確認）。
  - KOS の API 名（`arch_menu`・`net_init` など）：記憶に基づく推測で、KOS 2.3.0 のヘッダでは確かめていない（§10）。
  - 実機でしか分からないことは 11 件ある（§14）。重いのは次の3つ。
    1. `52 82` の後に BIOS のメニューに戻り、新しいディスクを起動できるか。
    2. `arch_reboot()` がハードリセットを出すか。出す場合、`reset_goto=1` だとメニューに戻ってしまう。
    3. イメージをマウントしたままセカンダリを読めるか。
- Opus による検証：一部（CI の範囲、ブランチの差分が設計書と索引の2ファイルだけであること）。
- 関連：R8（G2 設計）、R13・R14（`0x52`）、R48・R62（netgd-recv）、`docs/design/menu-app.md`。

### R98 2E のフラッシュ削減をオプトインで用意（Sonnet、Opus が差分を確認）

- 担当：Sonnet（実装）。
- 問い：2E の `resident,sd-fast` は余りが約 100 B しかない。既定の挙動を変えずに、削減案をすぐ使える形にできるか。
- 結論：
  - `mcu/netgd/Cargo.toml` にプロファイルを2つ足した（3c80ad3c・a4a20688）。ソースと feature は変えていない。
    - `release-z`：`opt-level="z"`。
    - `release-z-hot-s`：`release-z` のうち、`lzma`・`embassy-atsam3`・`psload` だけを `s` に戻す。
  - 使い方：`DEFMT_LOG=off cargo build --profile release-z --features sam3u2e,resident,sd-fast`。出力は `target/thumbv7m-none-eabi/release-z/netgd`。
  - 大きさ（text+data、`llvm-size`）：

    | ビルド | `release` | `release-z` | `release-z-hot-s` |
    |---|---:|---:|---:|
    | 2E `resident,sd-fast` | 122,268 | 114,588（−7,680） | 115,104（−7,164） |
    | 2E `sam3u2e` | 122,400 | 114,256 | 114,572 |
    | 4E 既定 | 163,112 | 147,468 | 147,756 |
    | board-v60 | 68,852 | 60,840 | 61,144 |

    2E の `resident,sd-fast` の余りは、約 7.3〜7.8 KB に増える。
  - 見送った案：
    - 案 3（`trim` の共用）：既定のビルドの生成コードが変わりうる。
    - 案 d（LZMA の辞書 8 KiB）：作業メモリが +4 KiB 増え、2E の SRAM0（16 KiB）を圧迫する。
    - 案 1（`list_root_dir`）：すでに feature `sd-list` の裏にあって、既定では入っていない。
  - `fwimage.py seal` は、両プロファイルの ELF で成功した。CI は `release` のパスを決め打ちしているので、CI では作らない。
- 根拠：
  - `docs/research/flash-budget-2e.md` §7。
  - 既定の5つの構成で、変更前後の bin と ELF の SHA-256 が一致した（同じ §7.4）。
  - `git diff main...` で、変更は `Cargo.toml` のプロファイル 24 行と文書だけ（Opus が確認）。
- 信頼度：大きさとバイト一致は確認済み。速度は未測定で、実機での比較が要る（IDE の応答、CD_READ の DMA、CDDA、FPGA の起動）。
- 注：4E の基準値（163,112）は R90 の値（162,692）と少し違う。測り方（`DEFMT_LOG` の有無など）の違いとみられる（推測）。2E と v60 は一致している。
- Opus による検証：差分の確認だけ。ビルドはし直していない。
- 関連：R81、R86、`docs/research/flash-budget-2e.md`。

### R99 Supra 2026.03 で AG16K 版をビルドし、ビットストリームの実サイズを測った（Fable、Opus が数字を確認）

- 担当：Fable（ツールの立ち上げ）。ユーザーの許可：「Supra を実行する許可します」。
- 問い：G1 で止まっていた「AG16K のビットストリームの実サイズ」（R66・R69・R81）はいくつか。埋め込めるか。
- 実行の環境：
  - Windows サンドボックスはこの PC に入っていない。有効にするとシステム設定の変更になるので、使わなかった。
  - 代わりに、Supra 2026.03 の **Linux 版**を Internet Archive から取得した。MD5 `6bf833a5…`・SHA-1 `edc93fee…` はアーカイブのメタデータと一致した。
  - WSL（Ubuntu 24.04）の中の、**ネットワークを切った名前空間（`unshare -rn`、`lo` だけで停止中）**で動かした。
  - 動かしたのは `af_cmd`・`af_setup`・`af_map`・`af_run` だけ。書き込み器・ドライバ・GUI・ライセンス関係は動かしていない。
  - 同梱のライセンスで、拒否されずに最後まで動いた（理由は調べていない）。ログに通信のエラーは無い。ただしシステムコールは追っていない（`strace` が無い）。
- 結論（詳細は `docs/research/ag16k-supra-build.md`）：
  - 資源：ロジック 2,291 / 16,624（13 %）、LUT 1,980、FF 1,089、M9K 14 / 56、PLL 1。
    **FIFO 5個はすべて M9K に載った**。R39・R32 の「非同期読み出しで LUT に膨らむ」懸念は、このフローでは起きない。
  - タイミング：48 MHz 系の Fmax は 90〜98 MHz、11.2896 MHz 系は 138〜175 MHz。
  - 非圧縮のビットストリームは 552,352 B（`_slave.rbf`）。
  - **`build.rs` と同じ LZMA の設定（`preset=9e,dict=4KiB,lc=0,lp=0,pb=0`）で 37,727 B**。
    - シードとクロックのピンを変えた3回で 37,727〜38,609 B。
    - R32 の見積もり（55〜75 KB）より約 17 KB 小さく、R81 のしきい値（約 53 KB）を約 15 KB 下回る。
    - `common/lzma` で展開すると、元と全バイト一致した。
  - AGM の圧縮（LZW、既定）は 255,887 B で、LZMA をかけても 71,381 B。**埋め込むなら「非圧縮を LZMA」一択**。
- 見つかった問題：
  1. 付属の `af_map.tcl` は inout を入力と出力に分け、`af` が別々のボールに置く。`.vqm` を後処理して `alta_io` にまとめる（`fixio.py`、32 組）のが必須。
  2. PLL は `alta_pllve`（`generate_pll` で生成）で通った。11.2896 MHz の入力に使えるピンは限られる（J4・T14 は通り、G16 は失敗）。
  3. `af_run` は、ピンが未割り当てのとき Error を出すのに、終了コード 0 で生成物を出す。
  4. 制約の約 9 % が効いていない。
- 根拠：
  - `ag16k-supra-build.md`。生成物とスクリプトは WSL の `~/supra-linux/work/`（リポジトリには入れていない）。
  - Opus が自分で `xz --format=raw --lzma1=preset=9e,dict=4KiB,lc=0,lp=0,pb=0 -c OpenGDEMU_slave.rbf | wc -c` を実行し、37727 を確認した。ファイルの大きさ 552,352 B も確認した。
- 信頼度：
  - 確認済み：大きさと LZMA の数字。
  - 推測：実機での動作（コンフィグ、PLL のロック、I/O、タイミング）は一切未確認。ピン配置は仮で、このビットストリームは基板に流せない。PLL の帰還の結線、`fixio.py` 後のネットリストは目視だけ。
- Opus による検証：あり（LZMA の大きさを独立に再計算した）。
- 関連：R32、R39、R66、R69、R81、R92、`docs/design/g1-ag16k-port.md`、`docs/research/ag16k-config.md`。

### R100 R89 A-c（ack でコマンドを取りこぼす）の修正と、slot 19・slot 5 の地雷の修正（Fable、Opus が確認）

- 担当：Fable（設計・デバッグ・実装）。コミットは b27f115c・971801bb。main への取り込みは cf3c3105。
- 問い：`ack_command` が `command_pend` を無条件に消すため、取り込みと ack の間に来たコマンドが消える（R89 A-c）。これを FPGA を変えて直せるか。
- 原因（窓は2つ）：
  1. ソフト：単純なハンドラは「Status を書く → ack」の順。ホストが BSY=0 を見て次のコマンドを書くと、直後の ack がそれを消す。
  2. ハード：ack のストローブが数クロック幅あり、同じクロックの Command の書き込みに勝つ。

  どちらの場合も、次のコマンドは BSY=1・pend=0・エッジなしで止まる。
- 修正（番号つき ack、`docs/design/mcu-fpga-handshake.md`）：
  - FPGA（`ide_device.py`）：
    - Command を書くたびに進む7ビットの `cmd_seq` を足した。スロット 0 を読むと、bit 14:8 が番号、bit 15 が 1。
    - 書き込みで bit 15=1 のときは、番号が一致し、かつ同じクロックに Command の書き込みが無いときだけ pend を消す。bit 15=0 は従来どおり。
  - ファーム（`ide.rs`）：
    - `take_command` は、スナップショットの前後でスロット 0 を読み、番号が一致するまで取り直す。
    - `ack_command` は、その番号を書き戻す。
  - 互換：古いビットストリームは上位バイトが 0 と読めるので、従来の ack になる。古いファームは bit 15=0 で書くので、従来の動作になる。新旧どの組み合わせでも動く。
  - slot 19：bit 12:8 を、ビットごとの書き込みマスクにした（0 なら従来どおり全部を書く）。中断の後の `disarm()` が、新しい PACKET の CDB の段階（PKT_PHASE）を消していた実害を直した。DMA モードとフロー制御は、あえてまとめて書くままにした（文書化）。
  - slot 5：MCU からの書き込みは下位バイトだけに届くようにした。
- 根拠：
  - 新しいシミュレーションのテスト6件（`test.py` :1033〜1262）。変更前の論理に当てると失敗し（`ack_keeps_command_written_after_snapshot` など）、変更後は通る。`& ~cmd_wr` を外すと失敗するので、ハードの窓も実際に試している。
  - **Opus が取り込み後の main で再実行した**：`test.py` 36、`test_cdda.py` 3、`test_timing.py` 5、`test_top.py` 5、`test_fifo.py` 10、すべて通過。`main.py`（両ターゲット）で作り直しても、`out/` に差分が出ない。
  - 2E `resident,sd-fast` は 122,112+156（変更前と同じ、Opus が確認）。
  - Fable の測定：4E は +20 B、2E と v60 は −460 B（インライン化の変化）。ホストテストは discimg 116・lzma 5・gamecfg 31 件で、失敗 0。
  - 2E `resident,sd-fast` の「残り約 100 B」は、512 B 単位に丸めた数字。512 B の境界まで実際に空いているのは 348 B（`.image_pad`）。
- 信頼度：シミュレーションとビルドは確認済み。実機は未確認。
- **注意**：同梱の `OpenGDEMU.rbf`（v5.5 用）は Quartus で作り直していない。作り直すまで、実機の FPGA は古い動作（従来の ack）のまま。
- まだ直していないこと：単純なハンドラは `complete()` を通らずに Status を書くので、BSY=1 の間に来たコマンドの BSY を、古い Status が消すことがある。コマンドそのものは失われず、次に処理される。
- 実機で見たいもの：BIOS の起動、DMA の CD_READ、WinCE のタイトル、DreamShell でのディスクの読み書き。新旧両方のビットストリームで確かめる。
- Opus による検証：あり（取り込み後のシミュレーション・ビルド・サイズ、`take_command`・`ack_command`・スロット 0 の論理を読んだ）。
- 関連：R89（A-c）、R90（573e95e5、レベルでの監視）、`docs/design/mcu-fpga-handshake.md`、`docs/AddressMapMCU.md`。

### R101 NetGD 専用メニュー `netgd-menu` の実装（設計の手順 1〜8）（Sonnet、Opus が確認）

- 担当：Sonnet（実装）。コミットは aecba5f5〜d618d5a1 の8つ。main への取り込みは c4cea201。
- 問い：R97 の設計の手順 1〜8 を実装し、ホストのテストと CI の KOS ビルドで確かめられるか。
- 結論：
  - `tools/netgd-menu/`（一覧・情報・起動・取り込み画面、NGTP サーバ、KOS 層）を作った。
  - `tools/netgd-recv` に足したもの：読み取り専用フラグ、軽いマウント、`fwctl` の6関数、模型の `0x52`・TEST UNIT READY。
  - `tools/netgd-send` に足したもの：`push`・`remote`（`ngtp.py`）。
  - mcu・fpga・`recv_core.c` は変えていない。
  - 設計からのずれ：
    - `nm_plat_controller()` を足した。
    - `NM_NET_AGAIN` を足した（届いていない状態と切断を分けるため）。
    - NGTP のエラー名（`EPROTO`・`EBADCRC`・`ESTATE`）を決めた。
    - 5分間通信が無いときに閉じる判定が、符号なしの引き算で誤って閉じていた不具合を直した。
  - **CI の KOS ビルド（KOS 2.3.0）**：
    - 全ファイルが、本物のヘッダで警告 0 でコンパイルできた。設計 §10 で未確認だった KOS の API 名（`arch_menu`・`net_init`・ソケットなど）は、少なくとも宣言の上では存在する。
    - リンクは失敗した。`Makefile` のオブジェクトの一覧に `ngtp_frame.o`・`ngtp_srv.o` が抜けていたため（44846460694 の ld の出力：`undefined reference to ngtp_srv_end` ほか）。Opus が一覧に足して直した。
- 根拠：Opus が取り込み後の main で再実行した。
  - `python tools/netgd-recv/test/run_tests.py`：52 件 OK（以前の 47 件 + 新しい 5 件）。軽いマウントの読み出しは 8 コマンド、FAT の全走査は 250 コマンド。
  - `python tools/netgd-menu/test/run_tests.py`：40 件 OK（U1〜U8・L1〜L14・N1〜N14。N7 は 12 か所で電源断を模擬）。
  - `tools/netgd-send` の `unittest`：67 件 OK。
  - CI の Build and Test（44846460649）は成功した。
- 信頼度：ホストのテストと KOS のコンパイルは確認済み。実機は未確認。
- 実機でしか分からないこと（設計 §14）：`52 82` の後に BIOS から起動できるか、`arch_reboot()` の動き、BBA なしで `net_init` が待たされないか、`make cdi` のツール名。
- Opus による検証：あり（テストの再実行、CI のログの確認、リンクの修正）。
- 関連：R97、`docs/design/menu-app.md`、`tools/netgd-menu/README_ja.md`。

### R102 Google Drive の成果物を 33545021 に更新（Opus）

- 担当：Opus。ユーザーの指示：成果物は Google Drive にも上げる（常設の指示）。
- 問い：R97〜R101 の成果（メニュー、番号つき ack、Supra でのビルド、release-z）を Drive に反映する。
- 結論：
  - R91 と同じやり方で、前の版（`firmware/`・`dreamcast/`・`docs/`・`はじめに読む.txt`）を `old-e9df6141/` に移した。削除はしていない。
  - 同じ構成で、新しい版（22 ファイル）を置いた。
    - `firmware/`：33545021 をこの PC で `DEFMT_LOG=off` のリリースビルドにしたもの。
      - `netgd-v55-4e`（bin 573,920 B）
      - `netgd-v55-2e`（bin 121,952 B）
      - `netgd-v60-2e-nobitstream`（bin 68,392 B）

      それぞれ `.bin` と `.elf`。
    - `dreamcast/`：KOS の CI の実行 44846846136（33545021、成功）の成果物 `dreamcast-elfs`。中身は `netgd-menu.elf`（新）・`netgd-recv.elf`・`netgd-fwupd.elf`・`gdprobe.elf`。
    - `docs/`：README・PLAN・LOG（R101 まで）・user-tasks.html・menu-app.md・netgd-menu の README_ja・mcu-fpga-handshake.md・ag16k-supra-build.md・flash-budget-2e.md・fable-debug・supra-static-analysis.md。
    - 最上位に、新しい `はじめに読む.txt`（R92〜R101 の変更点と、実機で最初に見てほしいこと）。
  - Supra のファイルと、仮のピン配置の AG16K ビットストリームは上げていない（再配布にあたる／基板に流せない）。
- 根拠：Drive が返した大きさが、手元のファイルと全部一致した。
- 信頼度：確認済み。
- Opus による検証：該当なし（Opus 自身の作業）。
- 関連：R91、R96。

### R103 v5.5 の FPGA ビットストリームを CI の Quartus で作り直した（Sonnet がワークフロー、Opus が実行・確認）

- 担当：
  - Sonnet：ワークフロー `.github/workflows/fpga.yaml` を書いた（a1a945ca）。
  - Opus：イメージを取れなかったときに切り替える処理を足し、CI で実行して rbf を取り込んだ。
- 問い：R100 の番号つき ack を実機で効かせるには、v5.5 の `OpenGDEMU.rbf` を作り直す必要がある。この PC には Quartus が無い。CI で作れるか。
- 結論：**作れた**。新しい rbf をリポジトリに入れた。
  - このフォークの GHCR には Quartus のイメージが無い（`quartus_container.yaml` をこのフォークで実行したことが無い）。
    上流の `ghcr.io/cepawiel/opengdemu/quartus2:latest` は、誰でも取得できる公開イメージだった（匿名のトークンで manifest が 200）。
    そこで、このフォークのイメージが無いときは上流のものを使うようにした。
  - 実行 44852741531（Quartus II 13.0 SP1、上流のイメージ digest `sha256:d468cf04…`）の結果：
    - Full Compilation は成功。エラー 0、警告 259。
    - 最悪の setup slack は +3.865 ns、hold は +0.215 ns。
  - 新しい rbf は 73,713 B（SHA-256 `6618252f…`）。前の rbf は 73,821 B（`04d9c2f7…`）。
    `build.rs` と同じ設定の LZMA では 48,772 B で、前の 49,629 B より 857 B 小さい。
  - ファームの大きさ（text+data、新しい rbf を入れたもの）：

    | ビルド | 大きさ |
    |---|---|
    | 4E | 161,708+144 |
    | 4E resident | 161,412+300 |
    | 2E | 120,924+156 |
    | 2E `resident,sd-fast` | 121,088+156 |

    2E の `resident,sd-fast` は約 1.1 KB 空いた。
- 根拠：CI のログ（上の slack の値と「Quartus II Full Compilation was successful」）、`sha256sum`、`xz … | wc -c`、`llvm-size`。
- 信頼度：ビルドは確認済み。**実機は未確認**。古い rbf は git の履歴に残っているので、実機で問題が出たら戻せる。
- Opus による検証：あり（ログ・ハッシュ・大きさ）。
- 関連：R100、`.github/workflows/fpga.yaml`、`.github/workflows/quartus_container.yaml`。

### R104 netgd-menu の実装のレビュー（Fable、読むだけ）

- 担当：Fable（独立したレビュー。リポジトリは変更していない）。
- 問い：実機に持ち込む前に、R101 の実装（`tools/netgd-menu`、netgd-recv への追加、netgd-send の `push`）に正しさのバグは無いか。
- 結論：実機の前に直すべきものが2つ、検討が要るものが1つ。書き込みの安全性（マウント中の書き込み、MBR、電源断）に新しい欠陥は無い。
  1. **（高）BBA があっても「BBA not found」になる**（`net_kos.c:27-34`、`main_kos.c:9`）。
     - KOS は `INIT_NET` が無いと、起動時に `bba_init()`・`la_init()` を呼ばない。ドライバが登録されないので、`net_init(0)` は -1 を返す。
     - 修正：`net_init` の前に `bba_init()`（必要なら `la_init()`）を呼ぶ。
     - 関連（低）：DHCP が失敗した後の再試行が無い。`net_dhcp_request` が最大 60 秒止まる間に、画面の表示が無い。
     - 信頼度：一次資料（上流の KallistiOS v2.1.0・v2.2.1・master のソース）。ユーザーのフォークそのものは読んでいない。
  2. **（高）`--slot auto`（既定）で同じコマンドをもう一度実行しても、再開にならない**（`push.py:99-109, 156-162`）。
     - 中断した途中のフォルダも「使用中」と数えられ、次の空き番号に全部を送り直す。途中のフォルダは残る。
     - 信頼度：確認済み（スクラッチの試験で、1回目は 03、2回目は 04 になり、`/03` に `DISC.NGP`・`NETGD.CKP` が残った）。
     - 修正：auto のとき、途中の状態の番号フォルダに先に `PUT resume` を試す。
  3. **（中、推測）メニューの TUR のポーリングが UNIT ATTENTION を消費する**（`nm_launch.c:52-63`、`ide.rs:1532-1548`）。
     - ファームは UA を BIOS に残す設計なので、BIOS が古い TOC のままになるおそれがある。
     - `52 82` が成功で返った時点で READY なので、TUR の待ちは要らないとみられる。
     - 修正案：BIOS に渡すときに TUR を送らないビルドオプションを用意し、実機（H2）で両方を試す。
- 問題なしと確認したもの：
  - NGTP のフレーム・CRC・長さ・状態が、C と Python で一致している。
  - ファームの `0x52` の応答とメニューの扱いが一致している。
  - `net_init` を無プロトタイプで呼んでも、SH4 の ABI では安全。
  - G1 のデバイスの選択、バッファの境界、書き込みの安全性。
- あとでよいもの：
  - A：黙って切れた接続が残り、新しい接続に応答しない（`ngtp_srv.c:317-327`）。
  - B：同じフレームで2つのボタンを同時に押したときの遷移（`nm_ui.c:593-601`）。
  - C：セッションの終了に失敗したときの扱いが、経路によって違う（`nm_ui.c:527-537`）。
  - D：「Stop safely」の後に、画面まで閉じてしまう。
  - E：EWOULDBLOCK のたびに `thd_sleep(1)` していて、速度の上限が約 800 KB/s とみられる（H6）。
  - F：番号が重複したとき、メニューの表示とファームがマウントするゲームが食い違いうる（H8）。
- 根拠：上記の `ファイル:行`。ホストの試験は全部通過（メニュー 40・recv 52・send 67）。再現スクリプトはスクラッチの `menureview/t_auto_resume.py`（リポジトリには入れていない）。
- Opus による検証：まだ（修正の段階で確かめる）。
- 関連：R97、R101、`docs/design/menu-app.md` §14。

### R105 古い Status が次のコマンドの BSY を消す競合を、番号つき Status で修正（Fable、Opus が確認）

- 担当：Fable（デバッグ・修正）。コミットは 5bc053c3（FPGA）・0cf783ad（ファーム）。main への取り込みは R105 のコミット。
- 問い：R100 で残した問題。単純なハンドラが書く古い Status が、その間に来た次のコマンドの BSY（PACKET なら CDB の DRQ）を消して、ホストが前のコマンドの結果を読んでしまう。これを直せるか。
- 原因（窓は2つ）：
  1. ソフト：Status を決めてから書くまでの間。
  2. ハード：MCU の書き込みのストローブが、同じクロックの Command の書き込みの `bsy.eq(1)` に勝つ。

  ハードの窓は、ファームだけでは閉じられない。
- 修正：
  - FPGA（スロット 1）：書き込みで bit 15=1 のときは、bit 14:9 が `cmd_seq` の下位6ビットと一致し、かつ同じクロックに Command の書き込みが無いときだけ、Status と INTRQ を反映する。
  - ファーム：`set_status`・`set_status_intrq` を `post_status` に通し、`ACK_TOKEN` から番号を付ける（`ide.rs:330-333`、Opus がビットの対応を確認：`(t << 1) & 0x7E00` で seq の下位6ビットを bit 14:9 に置く）。
  - 例外：`complete()` の「COMMAND_PEND が立っているのに Command が 0x00」の経路は、従来の形式のまま。
  - 互換：新旧のファームとビットストリーム（R100 より前を含む）の、どの組み合わせでも従来どおりかそれ以上に動く。
  - 番号は6ビットなので、間に Command がちょうど 64 回か 128 回書かれたときだけ、古い Status が通る。
- 根拠：
  - 新しいテスト3件（`stale_status_keeps_bsy_of_next_command`・`status_strobe_does_not_clear_bsy_of_command_write`・`status_write_forms`）は、変更前は失敗し、変更後は通る。
  - **Opus が取り込み後の main で再実行した**：`test.py` 39・`test_cdda` 3・`test_timing` 5・`test_top` 5・`test_fifo` 10 件、すべて通過。Verilog を作り直しても差分なし。
  - 2E `resident,sd-fast` は 121,608+156（R103 の新しい rbf を入れた状態）。
- 直していないこと：Error（スロット 3）・割り込み理由（スロット 5）・バイト数には番号を付けていない（新しいコマンドのハンドラが書き直す）。「Command=0x00 の latch」の原因は調べていない。
- 信頼度：シミュレーションとビルドは確認済み。実機は未確認。rbf は CI で作り直す（R107 の予定）。
- Opus による検証：あり（シミュレーションの再実行、`post_status` の読み、2E の大きさ）。
- 関連：R100、R103、`docs/design/mcu-fpga-handshake.md` §2.1。

### R106 Supra の AG16K ビルド手順をリポジトリに入れ、PS で流す時間を見積もった（Fable、Opus が中身を確認）

- 担当：Fable（ツールの統合・見積もり）。コミットは 293465c5・c5ec2cd1。main への取り込みは R106 のコミット。
- 問い：R99 の手順を誰でも再現できる形でリポジトリに入れられるか（Supra のファイルは入れない）。6.0 のファームにビットストリームを埋め込めるか。PS で流すのに何秒かかるか。
- 結論：
  - `fpga/agm/` を追加した：`build.sh`・`fixio.py`・`mkasf.py`・`mkpll.py`・`pll.tcl`・`src/OpenGDEMU.v`・`src/OpenGDEMU.sdc`・`pins/PROVISIONAL.asf`・`README_ja.md`。
    - `build.sh` は `SUPRA_HOME` が必須で、Supra を `unshare -rn` の中でしか動かさない。`unshare -rn` が使えなければ止まる（build.sh:59）。
    - PLL は、ビルドのたびに Supra の `generate_pll` で作る。その出力（`Audio_PLL.v`）はコミットしない（Supra の出力そのものなので）。
    - 仮のピンで作った出力は、必ず `OpenGDEMU_ag16k-provisional.rbf` という名前になり、ファームは拾わない。印の無いピンファイルは、66 ポートを1回ずつ割り当てていないと拒否する。
    - `fpga/agm/output/` は gitignore した。**Supra のツリーのファイルも、ビットストリームも、リポジトリに入っていない**（Opus が `git ls-tree` で確認）。
  - 再現（WSL、`unshare -rn`）：今のコアで `.rbf` 552,352 B、LZMA 39,857 B。LUT 2,027・FF 1,103・M9K 14。48 MHz 系の Fmax は 88.3 MHz。
    - **同じ入力でも、Supra の `.rbf` は毎回ビット単位では一致しない**（`af_run` の中で変わる）。LZMA 後の大きさは 36,684〜39,952 B の幅がある。見積もりには 40〜42 KB を使う。
    - `--seed 1` では配線が終わらなかったので、`--timeout`（既定 1,200 秒）を足した。
  - 埋め込み（39,857 B の rbf を一時的に置いて測った）：

    | ビルド | text+data | 122,368 B までの余り |
    |---|---:|---:|
    | `board-v60` | 112,236 | 10,132 |
    | `board-v60,resident,sd-fast` | 112,548 | 9,820 |
    | 同（`release-z`） | 104,872 | 17,496 |

    **R66 の「埋め込み」は、大きさの面で余裕をもって成り立つ**。測った後に rbf は消した。
  - PS で流す時間（見積もり）：
    - GPIO のビットバンで、1ビットあたり約 16 サイクル（96 MHz で DCLK は約 6 MHz）。
    - **AG16K は約 1.2〜1.7 秒、v5.5 は約 0.45〜0.7 秒**（+0.75〜1.0 秒）。
    - Dreamcast がどれだけ待てるかは不明（リポジトリにある根拠は「約 6 秒遅れると CDB-DRQ のエラー」だけ）。
    - PS の時間は、埋め込みでも SD でも同じなので、R66 の決め手にはならない。短くする案を5つ、`ag16k-supra-build.md` §13-4 に書いた（未実装）。
- 気づいた点（直していない）：`mcu/netgd/build.rs` の「`.bin` を置いた」という警告は先頭の `ff 55` を見ているが、Supra 2026.03 の `.bin` は `96 7e 3c 5a` で始まるので出ない。
- 根拠：`docs/research/ag16k-supra-build.md` §11〜§14、`fpga/agm/README_ja.md`。
- 信頼度：
  - 確認済み：手順の再現と大きさ。
  - 推測：PS の時間（公称のサイクル数で、フラッシュの待ちの効き方は未確認）。実機は一切未確認。
- Opus による検証：一部（リポジトリに入ったファイルの一覧、`pll.tcl` と `build.sh` の `unshare` の強制）。WSL での再ビルドはしていない。
- 関連：R66、R81、R99、`fpga/agm/`。

### R107 R105 の Verilog から作った rbf は、2E の `resident,sd-fast` に入らない（Opus）

- 担当：Opus。
- 問い：R105（番号つき Status）の FPGA の変更を、v5.5 の rbf に入れられるか。
- 結論：**CI でビルドはできたが、まだ入れていない**。2E の `resident,sd-fast` が 788 B 溢れるため。
  - 実行 44854550992（9c05b7d0）：Full Compilation は成功。setup slack は +3.792 ns、hold は +0.215 ns。
  - rbf は 74,685 B（SHA-256 `aaec5987…`）。LZMA で 51,372 B。R103 の rbf（73,713 B、LZMA 48,772 B）より 2,600 B 大きい。
    Quartus の圧縮済みの rbf は、配置によって大きさが変わり、圧縮済みのデータには LZMA があまり効かない。
  - 新しい rbf を入れた大きさ：
    - 2E `sam3u2e`：123,000+156（`resident` なしのフラッシュ 131,072 B には入る）。
    - 2E `resident,sd-fast`：上限 122,368 B を超えて、リンクに失敗する。`lazy-extents` を外しても失敗する。
    - `--profile release-z`（R98）なら 116,488+160 で入る。
  - CI（`build.yaml:72`）は `--release --features sam3u2e,resident,sd-fast` を作るので、この rbf を入れると CI が落ちる。
    そこで、リポジトリの rbf は R103 のもの（`6618252f…`）のままにした。
  - R103 の rbf と新しいファームの組み合わせでも動く。Status の番号は無視されて従来どおりになり、番号つきの ack（R100）は効く。
- 選択肢：
  1. 2E `resident,sd-fast` だけ `release-z` で作る（resident は配布していない開発用の構成）。
  2. Quartus のシードや設定を変えて、小さい rbf が出るまで試す。
  3. Quartus の圧縮を切り、LZMA に任せる（`build.rs` の注によると約 5 KB 小さくなるが、PS で流すビット数が倍になる）。
  4. R103 の rbf のままにする（今の状態）。
- 根拠：CI のログ、`xz … | wc -c`、`cargo build` のリンクのエラー、`llvm-size`。
- 信頼度：確認済み。
- Opus による検証：該当なし（Opus 自身の作業）。
- 関連：R98、R103、R105、`mcu/netgd/build.rs` の冒頭の注。

### R108 netgd-menu の任意の手順 9（UDP 探索）と 11（箱絵）を実装（Sonnet、Opus が確認）

- 担当：Sonnet（実装）。コミットは 80b0c081・1e4d5cd4。main への取り込みは 4b6b64f6。
- 結論：
  - 手順 9（`nm_disc.c`）：取り込み画面を開いている間、UDP 20039 で `NGTP-DISCOVER 1` に `NGTP-HERE 1 name=… app=… state=idle|busy` で答える。合言葉とポートは答えに入れない。
    PC 側は `netgd-send discover` を足した。`push`・`remote` で `--host` を省くと探索し、1台だけ見つかればそれを使う。
  - 手順 11（`nm_art.c`）：`NETGD.ART` は、16 B のヘッダ（`NGAR`、128×128）と RGB565 LE で、計 32,784 B。
    カーソルが 300 ms 止まったら、軽いマウントのまま 1 フレームに 4 KiB ずつ読む。読めなければ絵が出ないだけ。
    PC 側は `netgd_send/art.py`（Pillow は任意で、無くても内蔵の PNG 読み取りで作れる）と、`push --art`。
  - 決めたことは `tools/netgd-menu/README_ja.md` の「実装メモ」にある。
- 根拠：
  - Opus が取り込み後の main で再実行した：メニュー 58 件（2回とも OK）、recv 52、send 98。
    Sonnet は、メニューの全件で一度だけ 2 件が失敗したと報告している（再実行では通る。原因は不明で、負荷によるタイミングのずれとみられる）。
  - CI の KOS ビルド（44855310846）は成功した。UDP の `socket`/`bind`/`recvfrom`/`sendto` も、KOS 2.3.0 のヘッダでコンパイルできた。
- 信頼度：ホストのテストと KOS のコンパイルは確認済み。実機は未確認（H5 読み取りの時間、H10 右端に絵が収まるか）。
- Opus による検証：あり（テストの再実行、CI）。
- 関連：R97、R101、R104。

### R109 R104 の指摘の修正（Sonnet、Opus が確認）

- 担当：Sonnet（実装）。コミットは 74fc7ba8・4d3e59ec・c5d76002・eb0899d8。main への取り込みは 75da760f。
- 結論：
  1. BBA：`net_init` の前に `bba_init()` を呼ぶ。`la_init()` は、ヘッダがあるときだけ呼ぶ。
     2回目の Y で IP が 0 なら、DHCP をやり直す。DHCP の前に「Starting network ...」を表示する。
  2. `--slot auto`：`LIST /` で途中の状態（NETGD.CKP か `*.NGP`）の番号フォルダに、先に `PUT resume` を試す。EEXIST なら次へ進む。
     中断したら、`--slot NN` 付きの再実行のコマンドを表示する。レビューの再現は試験にした（2回目も同じ 03 で終わり、途中のファイルは残らない）。
  3. TUR：`NM_HANDOFF_TUR`（既定 0）を足した。既定では `52 82` が成功したら TUR を送らずに BIOS に渡し、UNIT ATTENTION を BIOS に残す。
     1 にすると従来どおり。実機の H2 で両方を試す（README_ja.md）。
  4. あとでよいもの：
     - A：黙って切れた接続を、新しい接続で置き換える。
     - B：1フレームに1つの遷移だけ。
     - C：セッションの終了に失敗しても、閉じていれば一覧に戻る。
     - D：ESTOP の後の B は、離すまで無視する。

     E（速度）と F（番号の重複）は README に書いただけ。
- 根拠：
  - Opus が取り込み後の main で再実行した：メニュー 76、recv 52、send 108（2回とも OK）。
  - CI の KOS ビルド（44857651127）は成功した。`bba_init`・`la_init`・`net_dhcp_request` も、KOS 2.3.0 のヘッダでコンパイルできた。
  - Sonnet は、netgd-send で一度だけ原因不明のエラーが1件出て、再実行では通ったと報告している。R108 のメニューの件と合わせて、ホストの試験に時々失敗するものがある（未調査）。
- 信頼度：ホストのテストと KOS のコンパイルは確認済み。実機は未確認。
- Opus による検証：あり（テストの再実行、CI）。
- 関連：R101、R104、R108。

### R110 Google Drive の成果物を 07458751 に更新（Opus）

- 担当：Opus。常設の指示（成果物は Google Drive にも上げる）による。
- 結論：
  - 前の版（33545021）を `old-33545021/` に移した。
  - 新しい版を同じ構成で置いた（23 ファイル）。
    - `firmware/`：07458751 をこの PC でビルドしたもの。v5.5 の rbf は R103 のもの。
      - `netgd-v55-4e`（bin 573,060 B）
      - `netgd-v55-2e`（bin 120,580 B）
      - `netgd-v60-2e-nobitstream`（bin 67,868 B）
    - `dreamcast/`：KOS の CI の実行 44857651127 の `dreamcast-elfs`。`netgd-menu.elf` は 2,916,608 B。
    - `docs/`：前回の分に、`netgd-send-README_ja.md` と `fpga-agm-README_ja.md` を足した。
    - 最上位に、新しい `はじめに読む.txt`（R103〜R109 の変更点）。
- 根拠：Drive が返した大きさが、手元のファイルと一致した。
- 信頼度：確認済み。
- 関連：R102。

### R111 R105 の rbf を入れ、2E の `resident,sd-fast` は `release-z` で作ることにした（Opus、ユーザーの判断）

- ユーザーの判断：「release-z で入れる」（R107 の選択肢 1）。
- 結論：
  - v5.5 の `OpenGDEMU.rbf` を、R105 の Verilog から CI（44854550992）で作ったものに差し替えた。
    - 74,685 B、SHA-256 `aaec5987…`。LZMA で 51,372 B。
    - 番号つきの ack（R100）と番号つきの Status（R105）が、両方とも FPGA で有効になる。
  - CI（`build.yaml`）の変更：
    - 2E `resident,sd-fast` の確認を `--profile release-z` にした。
    - 配布している 2E（`--release --features sam3u2e`）のビルドを足した。
  - 大きさ（text+data、`DEFMT_LOG=off`）：

    | ビルド | 大きさ |
    |---|---|
    | 4E | 122,988+156 |
    | 4E resident | 121,800+472 |
    | 2E | 123,000+156（フラッシュ 131,072 B に入る） |
    | 2E `resident,sd-fast`（release-z） | 116,488+160 |
    | v60 resident | 65,652+312 |
- 根拠：手元で、CI の5つの構成をすべてビルドした。`sha256sum`。
- 信頼度：ビルドは確認済み。実機は未確認。`release-z` の速度も未確認（R98）。
- 注意：配布している 2E のイメージは `release` のままで、残りは約 8 KB。Quartus の rbf は配置によって大きさが ±1 KB ほど変わる（R107）。
- 関連：R98、R103、R105、R107。

### R112 Google Drive のファームウェアを ee99eea2 に更新（Opus）

- 担当：Opus。常設の指示（成果物は Google Drive にも上げる）による。
- 結論：
  - R111 で v5.5 の rbf が変わったので、Drive の `firmware/` を作り直した。前の `firmware/` と `はじめに読む.txt` は `old-07458751/` に移した。
  - 新しい `firmware/`（ee99eea2、`DEFMT_LOG=off` のリリースビルド）：
    - `netgd-v55-4e`（bin 575,660 B）
    - `netgd-v55-2e`（bin 123,180 B）
    - `netgd-v60-2e-nobitstream`（bin 67,868 B、前と同じ内容）
  - `はじめに読む.txt` を R111 の内容に直した。
  - `dreamcast/` と `docs/` は R110 のまま（変わっていない）。
- 根拠：Drive が返した大きさが、手元のファイルと一致した。
- 信頼度：確認済み。
- 関連：R110、R111。

### R113 Drive の docs の LOG と PLAN を差し替え（R112 の訂正）と、実機をつなぐ PC への移行の確認（Opus）

- 担当：Opus。問いはユーザーの「すべて GitHub と Google Drive に上げたか」「実機をつなぐ PC で開発できるか」。
- 結論：
  - GitHub：`main` と `origin/main` はどちらも 8e75bf2f で、差はない（`git fetch` のあとの `git log origin/main..main` は空）。
  - Drive：R112 では「`docs/` は変わっていない」と書いたが、これは誤り。07458751 のあと、`docs/research/LOG.md` と `docs/PLAN.md` は変わっていた（R111・R112）。
    - この2つを差し替えた。古いものは `old-07458751/` に移した。
    - 新しい LOG は 262,287 B、PLAN は 31,520 B。Drive が返した大きさは、手元のファイルと一致した。
    - ほかの docs は 07458751 のあと変わっていない（`git log 07458751..HEAD -- docs` で確かめた）。
  - 実機をつなぐ PC への移行：
    - リポジトリに入っているもの：CLAUDE.md、LOG、PLAN、CI、ビルドの手順。`git clone` すれば同じルールで続けられる。
    - リポジトリに入っていないもの：
      - Claude の会話の記録と記憶。記憶はこの PC の `~/.claude/projects/…/memory`、3件。
      - Supra 2026.03 の tar。再配布しないためコミットしていない。SHA-256 は e37eb1d2…8197。
      - WSL の環境と KOS のツールチェイン。KOS は CI でビルドしているので、手元に無くてもよい。
    - 書き込みは `.cargo/config.toml` の runner（`probe-rs run --chip ATSAM3U4E --protocol swd`）で行う。2E と 6.0 では chip を変える必要がある。
- 根拠：上のコマンドの出力と、Drive の `create_file` の応答。
- 信頼度：確認済み。移行の部分は、手順を確かめた段階で、まだ試していない。
- 関連：R110、R112。

### R114 新しい PC で Claude を始める手順を Drive に置いた（Opus）

- 担当：Opus。ユーザーの指示（「新しい PC で Claude をどう始めるか、方法を書いたテキストを Drive に上げて」）による。
- 結論：Drive の NetGD フォルダの直下に、`新しいPCでClaudeを始める方法.txt`（3,436 B）を置いた。中身は次の5つ。
  1. アプリとアカウントの準備、`gh repo clone`。
  2. 最初に送る文。
  3. 記憶のフォルダのコピー（任意）。
  4. 自分で用意するもの（Pico と probe-rs、Supra の tar と SHA-256、BBA、KOS は CI でよいこと）。
  5. 会話は移らないことなどの注意。
- 根拠：Drive が返した大きさ（3,436 B）が、手元のファイルと一致した。
- 信頼度：確認済み（上げたこと）。手順そのものは、まだ試していない。
- 関連：R113。

### R115 新しい PC への移行と、6.0 の 7 ピンヘッダの導通確認（1 回目）（Opus）

- 担当：Opus。問いはユーザーの「新しい PC に引き継ぎたい（置き場所は E から H）」と「導通確認をしたい」。
- 結論（移行）：
  - リポジトリを `H:\OpenGDEmu` に置いた（`git clone`、be2c8370 = R114）。リポジトリ内の `E:\` はコメントや README の例だけで、書き換えは要らない。
  - この PC に入れたもの：rustup と stable 1.100（`thumbv7m-none-eabi`）、probe-rs-tools 0.32.0（crates.io からビルド）、`fpga/amaranth/amaranth_env`（amaranth 0.5.8、amaranth-yosys 0.50.0.0.post123、Python 3.12）。`gh` はログイン済み。
  - 確かめたこと：`mcu/netgd` の `cargo build --release` が通る。`test.py` 39 件、`tools/netgd-send` の unittest 108 件がパス。WSL にディストリビューションは無い（AG16K の作業のときに入れる）。
- 結論（導通、基板は無電源、SD カードなし）：
  - 6.0 の部品面のヘッダは 7 ピン（SD スロットの横の縦列。`docs/boards/gdemu60/README.md` の「8 ピン」は誤りで、7 ピンが正しい）、3 ピン（U1 の横）、5 ピン（基板の端、U2 側）、2 ピン（ロゴの横、何かがはんだ付けされている）。
  - 7 ピン（ユーザーの数え方。どちらの端を 1 にしたかは未記録）：7 = GND（SD スロットの外枠と導通）、5 = R1、4 = R2、3 = R3、2 = R4、1 と 6 = GND とも R1〜R4 とも導通なし。
  - これは v5.5 の H1（`docs/PCB.md:28-38`：1 = 3.3V、2 = TDI、3 = TDO、4 = TMS/SWDIO、5 = TCK/SWCLK、6 = NRST、7 = GND）と同じ形。7 ピンは U1 の 1 番ピン（SWD の 7・9・11 番がある辺）の角のすぐそばにある。
  - **仮説**：4 = SWDIO（R2 経由）、5 = SWCLK（R1 経由）、6 = NRST、1 = 3.3V、2 = TDI、3 = TDO。
- 未確認：MCU の 7 番・9 番が R2・R1 の先につながること（最後の決め手）。1 番が 3.3V であること。3 ピン・5 ピンの中身。
- 根拠：ユーザーのテスターでの測定（会話）。部品面・裏面の写真（会話、24 枚）。
- 信頼度：導通の結果は確認済み（ユーザーの測定）。SWDIO・SWCLK の割り当ては推測（v5.5 との形の一致による）。
- Opus による検証：写真と `docs/PCB.md` との照合のみ。
- 関連：`docs/boards/gdemu60/README.md`、`docs/research/web-prep-user-tasks.md`（タスク 2）、R24、R32、R44。

### R116 6.0 の SWD ピンを確定（R115 の仮説の確認）（Opus）

- 担当：Opus。問いは R115 の仮説（7 ピンの 4 = SWDIO、5 = SWCLK）が正しいか。
- 結論：**確定**。ユーザーが無電源の基板で、U1 の 7 番ピン（TMS/SWDIO）と R2 の穴と反対側のパッド、U1 の 9 番ピン（TCK/SWCLK）と R1 の穴と反対側のパッドの導通を確かめ、どちらも導通した。
  - 7 ピン：4 = SWDIO（R2 経由）、5 = SWCLK（R1 経由）、7 = GND。Pico の SWD はこの 3 本でつなぐ。
  - 1 = 3.3V、2 = TDI、3 = TDO、6 = NRST は v5.5 からの推定のまま（未確認）。SWD の読み出しでは 3.3V と NRST はつながない。
- 根拠：ユーザーのテスターでの測定（会話）。U1 の 1 番ピンの位置は、基板の白い丸印・チップのくぼみ・刻印の向きの 3 点で判断（写真）。
- 信頼度：確認済み（ユーザーの測定）。
- 関連：R115、`docs/boards/gdemu60/README.md`、`docs/PCB.md:28-38`。

### R117 純正 GDEMU 6.0 に gdprobe を初めて実行（switch=off、dcload-ip の CDI）（Opus）

- 担当：Opus。問いは P5.5 の 1 回目：純正ファームの GDEMU 6.0 に、読み取り系のコマンドだけを送ると何が返るか。
- 環境：Dreamcast + BBA（HIT-0400）+ PC の本体の LAN 端子（192.168.10.1。USB の LAN アダプタは外していた）。GDmenu から DCLOAD-IP を起動。
  gdprobe はこの PC の DreamSDK（KOS 2.3.0、gcc 13.2.0）でビルド。設定は `label=stock`、`switch=off`、`cdread_flags=0x20`。
- 結論：
  1. **1 回目は打ち切り**（`docs/research/probe-records/stock_20261121_01_abort.txt`）：最初の TUR から `rc=timeout`、STATUS と IRQ_REASON がどちらも `0xae`。
     原因は **G1 バスの保護が閉じたまま**だったこと。gdprobe は `KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` で起動するので、KOS の `cdrom_init` が行う
     「0x005F74E4 にサイズを書き、BIOS をバス越しに読む」手順が走らない。dcload-ip から起動した KOS プログラムでは、保護の状態（0x005F74EC）が 0 だった。
  2. **修正**：`tools/gdprobe/gdraw.c` に `gdraw_bus_prepare` を追加し、`main.c` がプローブの前に呼ぶ。状態が 3（通過）でなければ `cdrom_init` と同じ手順を行う。
     バスのコントローラだけの操作で、ドライブのレジスタには書かない。前後の値を記録のヘッダに残す。
  3. **2 回目は最後まで成功**（`stock_20261121_02.txt`、`abort=0`）：`g1bus prot_before=0 prot_after=3 bios_read=2097152 us=551392`。
- 純正の応答（今マウントされている dcload-ip の CDI に対して。`img=0`）：
  - **ディスクの形**：GET_TOC はトラック 2 本（1 = CTRL/ADR 0x01・FAD 150、2 = データ 0x41・FAD 11852、lead-out 0x05017C）。**REQ_SES_0 = `02 00 02 05 01 7c`（セッション数 2）**、
    REQ_SES_1 = FAD 150、REQ_SES_2 = FAD 11852（`002e4c`）、REQ_SES_3 = CHECK。トラック 2 本の CDI でセッション 2 を返す（D-1 で直した NetGD の動作の正解データ）。
  - **D-3**：REQ_STAT = `02 2f 01 01 01 00 00 96 00 00`。**バイト 1 の上位 4 ビットが 2**（ディスクの形式。CD-ROM XA）、下位が 0xF。NetGD はここに形式を入れていない（D-3 の指摘どおり）。
  - **D-2**：GET_SCD 形式 1 = `00 15 00 0e 01 01 01 00 00 00 00 00 00 96`。末尾の位置は **バイナリの FAD（0x000096 = 150）**。NetGD は BCD/MSF で返している（D-2 の指摘どおり、純正は仕様と同じ）。
  - **CD_READ のバイト 1**：`0x20` と `0x28`（Mode 2 Form 1）が 2048 バイト、`0x10` が 2352 バイト、**`0x24`（Mode 1）は CHECK・センス 5/64/00**。純正はセクタの種別を検査する。
    `0x20` と `0x28` の CRC は同じ（cdc10aa3）。
  - **識別**：REQ_MODE（0, 32）= `...SE      Rev 6.42990316`（実機の GD-ROM と同じ文字列）。GDEMU のバージョン（`52 00`）= `00 00 05 02 00 05 20 05`。
  - **速さ**：CD_READ 16 セクタずつ（PIO）64 回で **2,279 KiB/s**（1 回 13.9〜16.5 ms）。1 セクタのシーク読み 1.3〜2.9 ms。TUR・情報コマンドは 17〜33 µs。
- 未確認：GD-ROM の GDI（03）、CDI 2・3 本（04・05）、音声入り GDI（06）、空フォルダ（07）への切替。これは `tools/mksdcard` で試験用の SD を作ってから（`switch=on`）。
  `compare.py --manifest` による照合も、その SD を作ってから。
- 根拠：`docs/research/probe-records/stock_20261121_01_abort.txt`・`stock_20261121_02.txt`、`H:\DC\tools\dc-bba-tools\logs\auto_20261121_221553.txt`・`auto_20261121_221752.txt`、
  KOS の `kernel/arch/dreamcast/hardware/cdrom.c`（`cdrom_init`）と `include/dc/g1ata.h:121-143`。
- 信頼度：記録の値は確認済み（実機）。`0xae` が「保護が閉じているときの値」という解釈は、修正後に正常な応答に変わったことからの推論。
- Opus による検証：記録を読み、D-1〜D-3 の記述（`docs/PLAN.md`）と照合。
- 関連：`tools/gdprobe/README_ja.md`（「ホストテストでできていないこと」の 2・7）、`docs/research/stock-gdemu-flow.md`、R31、R55、R71。

### R118 純正 GDEMU 6.0 の記録（R117）と NetGD の予測を比較し、明確な差を純正に合わせた（Opus）

- 担当：Opus（サイドセッション、worktree `H:\OpenGDEmu-compare`、ブランチ `compare/stock-vs-netgd`、コミット `86a75a3c`）。メインの Opus がホストテストを再実行して確認し、ユーザーの指示で main に fast-forward で取り込んだ。
- 問い：R117 の純正の記録（dcload-ip の CDI、img=0）と、NetGD が同じディスクに返すはずの記録の違いはどこか。どれを純正に合わせるか。
- 入力：`docs/research/probe-records/stock_20261121_02.txt`。CDI は `H:\DC\gdemu_sd_cdpark\44\disc.cdi`（md5 `00175584cb3d54dbdc18375d131200ea`）。
  予測は `cargo run --release --example expect -- --image <cdi> --img 0 --no-ua --cdread-flags 0x20 --header --end`、比較は `compare.py --only-common`。
- 結論：差分 13 件（HASH 1047 行はすべて一致）。
  - 純正に合わせたもの：
    1. REQ_STAT バイト 1 の上位 4 ビット = SECNR と同じディスク形式（D-3。CDI の 0x2 は純正と一致、GDI の 0x8 は記録待ち）。
    2. GET_SCD 形式 1 の位置をバイナリ FAD に（D-2。相対位置は記録では 0 のため、仕様に合わせた。推測を含む）。
    3. GET_SCD 形式 0 のビット展開を Q = 0x40、P と R〜W = 0 に（D-5。Q のビット列は元から一致）。
    4. REQ_SES_0〜2 のバイト 0 = ドライブ状態（待機中 0x02）（D-5。旧 g1bios_replay の 0x01 は一時停止中と解釈。推測）。
    5. CDI の GET_TOC は area 1 でも同じ TOC を返す（R11 の判断を変更）。
    6. REQ_MODE は純正の 32 バイト（`…SE      Rev 6.42990316`）の範囲内の任意の窓を返す。範囲外と長さ 0 は従来どおり ABRT。
    7. REQ_ERROR = 先頭 F0 + 保留中のセンスキー。キーはコマンドごとに 0 に戻し、UA では 6 を明示する。
  - NetGD のまま：GDEMU_VERSION（NetGD 自身の互換版と機能ビットを返す方針）、CD_READ `0x10` の 2352 バイト（生セクタ読みは機能追加）、
    CD_READ `0x24` の拒否（データ 1 点で規則が分からず、拒否すると今動くものを壊す方向）。
  - データ待ち：CHECK のときのレジスタ値（純正は status = 0x11、err = センスキー << 4。全エラー経路に及ぶので別作業）、REQ_STAT バイト 1 の下位（純正は待機中 0xF）。
  - 比較の差分は 13 件 → 5 件。
- テストとビルド：`common/discimg` の `cargo test` 全通過（メインでも再実行して確認）、`tools/netgd-expect/test_expect.py` OK（10 件、1 件スキップ）。
  text は 4E 164,696 → 164,740（+44）、2E 123,000 → 123,136（+136）、2E `release-z` の `resident,sd-fast` 116,488 → 116,504（+16。イメージ 116,664 B、上限 122,368 B まで残り 5,704 B）。サイズはサイドセッションの報告の値。
- 根拠：`compare.py --only-common` の出力（前 13 件・後 5 件）、`common/discimg/tests/atapi.rs` の `the_stock_record_of_the_dcload_cdi`（記録のバイトをそのまま照合）。
- 信頼度：純正の値は確認済み（実機の記録）。NetGD 側はホストテストとビルドだけで、実機では未確認。
- Opus による検証：サイドセッションが記録と予測を 1 行ずつ照合。メインがコミットの差分の範囲とホストテストを確認。
- 未確認（GD-ROM の記録待ち）：D-4（GDI のセッション数）、GDI の REQ_STAT 0x8・lead-out・TOC area 1、Mode 1 トラックでの CD_READ の種別検査、REQ_ERROR の ASC。
- 関連：R11、R55、R71、R117、`docs/PLAN.md`、`common/discimg/src/atapi.rs`、`mcu/netgd/src/ide.rs`、`common/discimg/examples/expect.rs`。

### R119 純正 GDEMU 6.0 でイメージを切り替えて調べた（mksdcard の試験用 SD、03〜07）（Opus）

- 担当：Opus。問いは P5.5 の 2・3 回目：純正は `52 82 n` の切替にどう応え、GD-ROM の GDI・CDI・空フォルダに何を返すか。
- 環境：R117 と同じ（Dreamcast + BBA + PC の本体の LAN 端子）。SD は別のカード（119 GB、MBR・FAT32）に `tools/mksdcard` の出力を書いたもの
  （`H:\NetGD_probe_sd`。01 = GDmenu v0.6、02 = dcload-ip（`H:\DC\gdemu_sd_cdpark\44\disc.cdi`）、03〜06 = 合成イメージ、07 = 空、ルートに GDEMU.ini）。
  書き込み後に全ファイルの大きさを照合（一致）。`verify.py` でも 03〜06 がマニフェストと一致。
- 記録：`docs/research/probe-records/stock_20261121_03_img3.txt`（`images=3`）、`stock_20261121_04_img3-7.txt`（`images=3,4,5,6,7`）。どちらも `abort=0`、`return_to=2` で 02 に戻った。
  `compare.py --manifest` で **HASH 1,050 件・1,056 件すべて一致**（純正の読み出しにデータ化けなし）。
- 結論：
  - **切替の手順**（03・04・05・02 で同じ）：`52 82 n` は即座に完了（status 0x50）。そのあと TUR が **2/3A/00（ディスクなし）→ 約 150 ms で 2/04/01（準備中）→ 約 300 ms で 6/28/00（ディスクが替わった）→ 約 335〜350 ms で READY**。
  - **D-4（GD-ROM のセッション数）**：03 の REQ_SES_0 = `01 00 01 00 fe 7e`（**セッション 1**、lead-out FAD 65150）、REQ_SES_1 = FAD 150、REQ_SES_2・3 は CHECK。
    NetGD の今の動作（1）と同じで、Flycast（2）とは違う。**D-4 は NetGD のままでよい**。
  - **GD-ROM の REQ_STAT**：`01 8f 41 03 01 00 b0 5e 00 00`。バイト 1 の上位 = **8**（R118 で仕様から入れた値が正しかった）、CTRL/ADR 0x41・トラック 3・インデックス 1・FAD 45150。
    バイト 0 のドライブ状態は 1（一時停止）。R117 の 2 回目は 2（待機）で、状態によって変わる。
  - **GD-ROM の TOC**：area 0 = トラック 1（0x41、FAD 150）と 2（0x01、FAD 906）、area 1 = トラック 3（0x41、FAD 45150）だけ（area 0 の分は FF）。
  - **CD_READ の種別検査**：GD-ROM（Mode 1）では `0x24` が通り `0x28` が 5/64/00。CDI（Mode 2 Form 1）では逆。`0x20` はどちらも通り、`0x10` はどちらも 2352 バイト。
    **純正は期待セクタ種別を検査する**（規則が 2 つのイメージで確かめられた）。
  - **CDI（04 = 2 トラック、05 = 3 トラック）**：どちらもセッション 2 個。05 の TOC はトラック 2（音声）を **FAD 602**（mkdcdisc の記述部の `start_lba` 452 + 150。pregap の頭）として返す。
    NetGD のパーサと同じ扱い（`tools/mksdcard/README_ja.md` の「限界」）。CDI では area 0 と area 1 に同じ TOC（R118 の変更と一致）。
  - **06（低密度領域に音声 3 本の GDI）は純正がマウントしない**：2/3A/00 → 2/04/01 → **2/3A/00 のまま**で 15 秒のタイムアウト。そのあと情報コマンドはすべて CHECK（センスキー 2）。
    03（音声 1 本）は通るので、**低密度領域のトラックが 3 本以上の GDI を純正は受け付けない**可能性が高い（原因は未確定。GDI の書式はどちらも同じ形）。NetGD は受け付ける。
  - **07（空フォルダ）**：06 と同じく 2/3A/00 のままでタイムアウト（ディスクなしの扱い）。そのあと 02 への切替は普通に成功し、純正は固まらない（R18 の欠陥 1 の参考）。
  - **速さ**：03 の CD_READ 16 セクタずつで 2,278〜2,285 KiB/s（R117 の CDI と同じ）。**GET_SCD 形式 1 だけ約 24 ms かかる**（ほかの情報コマンドは 17〜34 µs）。
- 設定の不備：`cdread_flags` を既定（0x24）にしたので、04・05（Mode 2）では IP_READ とスループット試験が CHECK で飛んだ（`TP_SUMMARY ok=0`）。データの照合は CDREAD_FLAGS の行で済んでいる。
  CDI の速さを測るなら `cdread_flags=0x20` で再実行する。
- 未確認：06 を拒む理由（トラック数か、低密度領域の配置か）。CHECK のときの G1 バスのレジスタ値の扱い（R118 の「データ待ち」のまま）。
- 根拠：上の 2 つの記録、`compare.py --manifest H:\NetGD_probe_sd\NETGD-PROBE-MANIFEST.json <記録>` の出力（`hashes: 1050 match` / `1056 match, 0 mismatch`）、`H:\NetGD_probe_sd\06\disc.gdi`・`03\disc.gdi`。
- 信頼度：記録の値は確認済み（実機）。06 の理由と、ドライブ状態 1/2 の意味は推測。
- Opus による検証：記録を読み、R117・R118 と PLAN の D-1〜D-5 と照合。
- 関連：R18、R71、R117、R118、`tools/mksdcard/README_ja.md`、`docs/research/stock-gdemu-flow.md`。

### R120 純正 GDEMU 6.0 の CDI の読み出し速度を測り直した（R119 の設定の不備の補い）（Opus）

- 担当：Opus。問いは、R119 で測れなかった CDI（04・05）の CD_READ の速さ。
- 設定：`images=4,5`、`return_to=2`、**`cdread_flags=0x20`**（R119 は 0x24 のままで、Mode 2 の CDI では純正に CHECK で断られていた）。SD は R119 と同じ。
- 記録：`docs/research/probe-records/stock_20261121_05_cdi-tp.txt`（`abort=0`）。`compare.py --manifest` で **HASH 2,098 件一致、不一致 0**、2 件は照合できず（下の postgap）。
- 結論：
  - **速さは GD-ROM と同じ**：CD_READ 16 セクタずつ 64 回で 04 = 2,273 KiB/s、05 = 2,281 KiB/s（1 回 13.9〜16.5 ms、中央値 13.98 ms）。
    R117（dcload-ip の CDI）・R119（GD-ROM の GDI、2,278〜2,285 KiB/s）と変わらないので、**純正の速さはイメージの種類によらず、1 回 32 KB の PIO で約 14 ms** と見てよい。
  - 1 セクタのシーク読みは 1.3〜3.1 ms。切替は 04・05・02 とも約 0.35 秒で READY（R119 と同じ）。
  - **postgap のセクタは 2,324 バイトで返る**：シーク試験の最後の点（04 = FAD 15853、05 = FAD 16405。lead-out の直前、mkdcdisc が置く 2 セクタの postgap）を
    `0x20` で読むと、純正は **2,324 バイト（Mode 2 Form 2 のユーザーデータの大きさ）**を返した。マニフェストに無い領域なので照合はできない。
    NetGD がこのセクタに何バイト返すかは未確認（次に比較するときの項目）。
- 信頼度：確認済み（実機の記録）。「速さはイメージによらない」は 4 種類のイメージ（dcload-ip の CDI、03、04、05）での観察。
- Opus による検証：記録を読み、R117・R119 の値と比較。
- 関連：R117、R118、R119、`tools/mksdcard/README_ja.md`（postgap の注意）。

### R121 純正が 06 の GDI を拒む理由の切り分け（08・09 を追加）（Opus）

- 担当：Opus。問いは R119 の「06（低密度領域に音声 3 本、高密度領域のデータがトラック 5）を純正がマウントしない」理由。
- 仮説：A = 低密度領域のトラックが 3 本以上（高密度領域がトラック 3 から始まらない）だと拒む。B = トラックの総数が 4 本以上だと拒む。
- 方法：`tools/mksdcard` に `--set variants` を追加し、2 つの GDI を `H:\NetGD_probe_sd_v2` に作った（`verify.py` でマニフェストと一致、`tests/test_mksdcard.py` 17 件パス）。
  - `08`：低密度領域 = データ（LBA 0）+ 音声（756）、高密度領域 = データ（45000、IP.BIN）・音声（55150）・データ（55900）。全 5 トラック。
  - `09`：低密度領域 = データ（0）+ 音声（756）+ 音声（1506）、高密度領域 = データ（45000、IP.BIN）。全 4 トラック。
  ユーザーが試験用 SD に `08`・`09` をコピーした（SD 上の大きさの照合はしていない）。gdprobe は `images=3,6,8,9`、`return_to=2`。
- 記録：`docs/research/probe-records/stock_20261121_06_gdi-variants.txt`（`abort=0`）。`compare.py --manifest` を 2 つのマニフェストで実行し、
  03 と 08 の HASH はそれぞれのマニフェストと**すべて一致**（1,050 件ずつ、不一致 0）。
- 結論：
  - **03 は受け付け**（基準。R119 と同じ値）、**06 は今回も拒否**（2/3A/00 → 2/04/01 → 2/3A/00 で 15 秒のタイムアウト。再現する）。
  - **08 は受け付け**：切替は約 0.35 秒で READY。TOC の area 1 = トラック 3（0x41、FAD 45150）・4（0x01、FAD 55300）・5（0x41、FAD 56050）。
    REQ_SES_0 = `01 00 01 01 02 02`（セッション 1、lead-out FAD 66050）。**トラックの総数が 5 本でも問題ない**（仮説 B は否定）。
  - **09 は拒否**：06 と同じ並び（2/3A/00 → 2/04/01 → 2/3A/00）で 15 秒のタイムアウト。
  - したがって **純正は、低密度領域のトラックが 2 本（トラック 1 = データ、トラック 2 = 音声）で、高密度領域がトラック 3 から始まる GDI だけを受け付ける**（仮説 A）。
    実際の GD-ROM の構成どおりで、それ以外の GDI はディスクなしとして扱う。
- NetGD への影響：NetGD は 06・09 も受け付ける（パーサは構成を問わない）。市販の GD-ROM から作った GDI はこの形を外れないので、互換性の問題にはならない。
  純正に合わせて拒むかどうかは、今は決めない（拒む利点が無い）。
- 注意：09 は SD 上のファイルを照合しておらず、読み出しも無いので、コピーの失敗で拒まれた可能性は残る。06（PC でコピーし、大きさを照合済み）が同じ結果なので、可能性は低い。
- 根拠：上の記録、`H:\NetGD_probe_sd_v2\08\disc.gdi`・`09\disc.gdi`、`compare.py --manifest` の出力。
- 信頼度：08 の受け付け・06 の拒否は確認済み。09 の拒否は確認済み（ただし上の注意）。「規則」は 4 つの GDI（03・06・08・09）からの推論。
- Opus による検証：記録を読み、R119 と照合。
- 関連：R119、`tools/mksdcard/mksdcard.py`（`gdi_spec_08`・`gdi_spec_09`・`make_variants`）、`tools/mksdcard/README_ja.md`。

### R122 R121 の 09 の再確認（SD 上のファイルを照合してから）（Opus）

- 担当：Opus。問いは R121 の注意点：09 が拒まれたのは、SD へのコピーの失敗のせいではないか。
- 方法：試験用 SD を PC に挿し、`H:\NetGD_probe_sd_v2` の `08`・`09` を上書きでコピーし直し、全 11 ファイルの MD5 を元と照合（すべて一致）。
  そのあと gdprobe を `images=9`、`return_to=2` で実行。
- 記録：`docs/research/probe-records/stock_20261121_07_img9-recheck.txt`（`abort=0`）。
- 結論：**09 は今回も拒否**。2/3A/00 → 約 150 ms で 2/04/01 → 約 300 ms で 2/3A/00 に戻り、15 秒のタイムアウト（R121 と同じ）。02 への切替は約 0.34 秒で READY。
  中身が正しい 09 でも拒まれるので、**R121 の規則（低密度領域はトラック 1・2 だけ、高密度領域はトラック 3 から）は確定**とする。
- 根拠：上の記録、MD5 の照合（11 ファイル、不一致 0）。
- 信頼度：確認済み。
- 関連：R121。

### R123 R119〜R122 の純正の記録と NetGD の予測の比較（GD-ROM・CDI・空フォルダ・切替）（Opus）

- 担当：Opus（サブエージェントが比較、メインが確認）。問いは R118 で比べなかった記録（03〜09 と切替）で、純正と NetGD の差はどこか。
- 方法：main（43c5f4b4）の `common/discimg/examples/expect` と `tools/netgd-expect/expect_sd.py` で同じイメージの予測を作り、
  `compare.py --only-common` で `stock_20261121_04_img3-7.txt`・`05_cdi-tp.txt`・`06_gdi-variants.txt` と比較。切替（`SW_*`）は `ide.rs` を読んで手で比較。
  生の差分は 246・26・88 行。同じ原因から出る行をまとめると 15 件。
- 差（D1〜D15）：
  - D1 **GD-ROM の lead-out**：純正は最後のトラックの本当の終わり（03 = FAD 65150、08 = FAD 66050）。NetGD は市販ディスクの大きさ（FAD 549300、`GDROM_LEADOUT_LBA`）まで埋める。
    読めない範囲を「ある」と答えることになる（シーク試験の差はすべてこれが原因）。**新規**。
  - D2 REQ_STAT・REQ_SES のバイト 0：純正は切替のあと 01（一時停止）、NetGD は 02（待機）。R117 では純正も 02 で、状態による。
  - D3 REQ_STAT バイト 1 の下位（繰り返し回数）：純正 F、NetGD 0。
  - D4 GD-ROM の待機中のヘッド位置：純正はトラック 3・FAD 45150、NetGD はトラック 1・FAD 150（CDI はどちらもトラック 1・FAD 150）。
  - D5 CHECK のレジスタ：純正は status 0x11・err = センスキー << 4。NetGD は 0x51、範囲外で err 0x04（ABRT）など（R118 の「データ待ち」）。
  - D6 範囲外のセッション：純正は CHECK・センスキー 5、NetGD は ABRT でキーは 0。
  - D7 **REQ_ERROR の ASC/ASCQ**：純正は 3A/00・04/01・28/00・64/00 を入れる。NetGD は常に 0。
  - D8・D9 CD_READ の種別検査（R118 で保留のまま）、D10 `0x10` の生セクタ（同）。
  - D11 **06・09 の GDI**：純正はマウントしない（R121）、NetGD はマウントし、低密度領域の音声トラックを area 1 の TOC に入れる。**新規**、方針の問題。
  - D12 **空フォルダ（07）**：純正は切替を受け付けて「ディスクなし」になる（あとの切替で戻る）。NetGD は `52 82 07` を断り（0x51/0x54、センスキー 5）、前のディスクがマウントされたまま。
  - D13 **切替のあとの NOT READY**：純正は 2/3A/00 → 約 150 ms 2/04/01 → 約 300 ms 6/28/00 → 約 340 ms READY。NetGD は即座に UNIT ATTENTION 1 回のあと READY
    （`ide.rs` の `APPLY_SWAP_WINDOW = false`。この仕組みは「純正のデータ待ち」で止めてあり、オンにする条件がコメントに列挙されている）。
  - D14 CDI の postgap のセクタ：純正 2,324 バイト、NetGD 2,048 バイト。
  - D15 GDEMU_VERSION（R118 で NetGD のままと決定）。
- 一致したもの：REQ_MODE の 3 つの窓、何も無いときの REQ_ERROR、GD-ROM の GET_TOC area 0（03・08）、area 1 のトラック（lead-out 以外）、CDI の TOC（両方の領域）、
  GD-ROM のセッション数 1・CDI の 2、REQ_STAT のディスク形式、CDI の GET_SCD、IP_READ、各フラグでの CRC、HASH と TP のデータ（postgap の 2 件以外）。
- 判断：D1・D3・D7 は純正に合わせる（R125 で実装。R124 で見つかった IDENTIFY の差 D16 も同時に）。D5・D13 はエラーを返すすべての経路に関わるので別作業。D12 は方針をユーザーと決める。D2・D4・D6・D14 は影響が小さいので保留。
- 根拠：サブエージェントの比較の出力（scratchpad の `compare2\cmp_*.txt`）、`common/discimg/src/atapi.rs:23-86`、`mcu/netgd/src/ide.rs:1340-1366`。
- 信頼度：純正の値は確認済み（実機）。NetGD 側はホストでの予測（`expect`）とソースの読み取りで、実機では未確認。
- 関連：R18、R117〜R122、R118。

### R124 純正 GDEMU 6.0 の IDENTIFY PACKET と、時間の値のぶれ（Opus）

- 担当：Opus。問いは (1) 純正は ATA の IDENTIFY PACKET DEVICE（0xA1）に何を返すか、(2) R119〜R122 の時間の値は実行ごとにどれだけぶれるか。
- 方法：gdprobe を `identify=on`、`images=3`、`return_to=2` で実行（R119 の 1 回目と同じ条件 + IDENTIFY）。
- 記録：`docs/research/probe-records/stock_20261122_08_identify.txt`（`abort=0`、HASH 1,050 件一致）。
- 結論：
  - **IDENTIFY PACKET は 80 バイト**（status 0x50）：バイト 0〜2 が 0x20、3〜15 が 0、16〜31 = `SE`、32〜47 = `CD-ROM DRIVE`、48〜63 = `6.42`（それぞれ空白で 16 バイトに埋める）、64〜79 が 0。
    本物の GD-ROM ドライブの形式（ATA 標準の 512 バイトではない）。**NetGD は標準の 512 バイト（機種 `OpenGDEMU GD-ROM`、版 `1.00`）を返していて違う（D16、新規）**。
  - **時間のぶれ**（03 を 4 回：R119 の 2 回、R121、今回）：切替から READY まで 331〜341 ms、CD_READ 16 セクタの中央値 13.999〜14.010 ms（スループット 2,278〜2,285 KiB/s）、
    シーク読みの中央値 1.73〜1.77 ms・最大 3.16〜3.26 ms、GET_SCD 形式 1 は 24.4〜24.9 ms。**どれも 1〜3 % 以内で、基準値として使える**。
- 根拠：上の記録と R119・R121 の記録（`stock_20261121_03_img3.txt`・`04_img3-7.txt`・`06_gdi-variants.txt`）の img=3 の行。
- 信頼度：確認済み（実機）。
- 関連：R119、R121、R123、`mcu/netgd/src/ide.rs` の `stage_identify_packet`。

### R125 純正の応答 4 件を NetGD に採用（D1 lead-out・D3 繰り返し回数・D7 ASC/ASCQ・D16 IDENTIFY）（Opus）

- 担当：Opus（サブエージェントが実装、メインがテストの再実行と差分の範囲を確認）。worktree `H:\OpenGDEmu-stock2`、ブランチ `stock/leadout-asc-repeat`、コミット `383a2ccb`（main 43c5f4b4 から）。
- 問い：R123・R124 の差のうち、純正の動きがはっきりしていて影響が小さい 4 件を純正に合わせる。
- 結論：
  - **D1**：GDI の lead-out を「最後のトラックの開始 + セクタ数」にした（セクタ数はトラックファイルの大きさ ÷ セクタ長。`gdi.rs` の `parse_image` と `expect.rs` の既存の処理）。
    TOC area 1 の FAD 549300 への水増しをやめ、REQ_SES(0) も同じ規則。`GDROM_LEADOUT_LBA` はトラック表が空のときの予備値だけに残す。lead-out の CTRL は最後のトラックのもの（03 で 0x41、純正と一致）。CDI は変更なし。
    シーク・読み出しの範囲チェックはトラックごとのセクタ数を使っていて、この定数は使っていなかった。
  - **D3**：CD-DA を再生していないとき REQ_STAT バイト 1 の下位を 0xF に。再生中は繰り返し回数のまま（純正の再生中の記録は無い）。
  - **D7**：REQ_ERROR のバイト 8・9 に ASC/ASCQ。`DeviceState::pending_asc` と `medium_changed()`（マウント後の UA = 6/28/00）・`not_ready()`（メディア無し 2/3A/00、準備中 2/04/01）・`clear_sense()`。
    MEDIUM ERROR（キー 3）と ILLEGAL REQUEST（キー 5）の ASC は純正の値が分からないので 0 のまま。起動時の UA も 28/00 にした（純正の電源投入時の値は不明）。
  - **D16**：IDENTIFY PACKET を純正の 80 バイト（`IDENTIFY_PACKET_REPLY`、バイト数 80）に。転送後 `finish_pio_in` で DRQ を落とす（無いと gdprobe が 80 バイトを超えて読み続ける）。セカンダリの IDENTIFY DEVICE（0xEC）は変更なし。
- テスト：PLAN の決まりどおり期待値を先に純正の値に。GDI 03・08 の TOC の末尾と REQ_SES_0、メディア無しの REQ_ERROR（`f0 00 02 00 00 00 00 00 3a 00`）、REQ_STAT の `8f`、IDENTIFY の 80 バイトを固定。
  `common/discimg` の `cargo test` 全通過（メインでも再実行）、`test_expect.py` OK（10 件、1 件スキップ）。
- 比較（`stock_20261121_04_img3-7.txt`、`--only-common`）：246 → 230 行。D1（GET_TOC_1、REQ_SES_0、SEEK_01〜15）と D3 の差は消えた。SW_POLL の差は D13（切替のあとの NOT READY）なので残る。
- 大きさ（text + data、43c5f4b4 → 383a2ccb）：4E −300、2E（`DEFMT_LOG=off`）−32、2E `release-z` の `resident,sd-fast` ±0（116,660 B、上限 122,368 B）。512 バイトの IDENTIFY を組み立てる処理が無くなった分減った。
- 未対応：IDENTIFY のあとの IRQ_REASON（純正 0x03、未設定）、メディアが無いときの CD_READ でセンスキーを立てない（以前から）、REQUEST SENSE（0x03）の ASC。続きの作業（D13・D5・D12）で扱う。
- 信頼度：ホストテストと比較で確認済み。実機では未確認。
- 関連：R123、R124、`common/discimg/src/atapi.rs`、`common/discimg/tests/atapi.rs`、`mcu/netgd/src/ide.rs`、`mcu/netgd/src/gdi.rs`、`common/discimg/examples/expect.rs`、`tools/netgd-expect/test_expect.py`。

### R126 切替のあとの NOT READY・CHECK のレジスタ・中身の無いフォルダを純正に合わせた（D13・D5・D12）（Opus）

- 担当：Opus（サブエージェントが実装、メインがホストテストを再実行して確認）。ブランチ `stock/leadout-asc-repeat`、コミット `805e98b4`（親 383a2ccb）。3 件は共通の関数（`finish_check`）に依存するので 1 コミット。
- **見つかったこと**：試験用 SD の `GDEMU.ini` は `open_time = 150`、`detect_time = 150`（GDmenu v0.6 の雛形）。純正で測った「約 150 ms + 約 150 ms」は、この INI の値そのものである可能性が高い。
  キーが無いときに純正がどれだけ待つかは未測定（公式の既定値とされる 500 / 250 も未確認）。
- 結論：
  - **D13（切替の窓）**：マウントを試すたび（`52 82`・`52 81`・ボタン・0xE0 の MOUNT・LOAD・WSESSION_END）に窓を開く。
    open 段階 = 2/3A/00、detect 段階 = 2/04/01、窓のあとの最初のコマンドで 6/28/00 を 1 回、そのあと READY。イメージが無ければ窓のあとも 2/3A/00。
    時間は、GDEMU.INI がキーを設定していればその値、無ければ純正の実測値の 150 + 150 ms（`discimg::atapi::SWAP_OPEN_MS`・`SWAP_DETECT_MS`）。`swap_delay_ms` は `Ini::swap_window_ms` に置き換え。
    窓の中とディスクなしのときの答えは `not_ready_answer`（純正の 06・07 の記録どおり）：TUR・REQ_STAT・GET_TOC・REQ_SES 0・GET_SCD・CD_READ・CD_PLAY は NOT READY（0x11/0x20）、
    REQ_SES 1 以上は ILLEGAL（0x11/0x50）、REQ_MODE・REQ_ERROR・0x52・0xE0 などは通常どおり。判定は `handle_packet` の入口の 1 か所（`drive_not_ready`）。
    SECNR の状態は open（6）→ busy（0）→ pause（1）か no disc（7）（**推測**。gdprobe は SECNR を記録しない）。ハードリセット・EJECT・WSESSION_BEGIN で打ち切る。
    `APPLY_SWAP_WINDOW`・`READY_AT`・`media_ready` は削除。`expect` は時間を模擬せず、状態の変わる行を並べる（`t_ms` は `-`）。
  - **D5（CHECK のレジスタ）**：パケットデバイスの ATAPI の CHECK はすべて status 0x11・err = センスキー << 4、センスキーを保留（`finish_check`・`finish_illegal`）。
    REQ_SES の範囲外はキー 5。0x52・0xE0 の拒否は 0x54 → 0x50、MEDIUM ERROR は 0x34 → 0x30。ATA の ABRT（IDENTIFY DEVICE、未知の ATA コマンド）とセカンダリ（ディスク）は変更なし。
    IDENTIFY PACKET のあとに IRQ_REASON 0x03（R125 の残り）。
  - **D12（中身の無いフォルダ）**：番号のフォルダはあるがマウントできるイメージが無い（または解析に失敗する）とき、`52 82 n` を 0x50 で受け付けてディスクを外し、窓のあとは「ディスクなし」。
    `load_by_name` がフォルダを先に `remember()` するので、`52 81` とボタンは先へ進める。`EMPTY_SLOT` と `discimg::slot::swap_allowed` でボタンが効き続ける（R18 の欠陥 1 の再発防止。EJECT・WSESSION・ディスクなしでの最初のディスク書き込みで解除）。
    番号のフォルダが無いときは従来どおり断り、前のイメージを残す。純正が拒む GDI の形（D11）は受け付けたまま。
- テスト：期待値を先に純正の値にして失敗を確かめてから実装（PLAN の決まり）。`common/discimg` 131 件、`common/gamecfg` 31 件、`common/lzma` 5 件、`test_expect.py` 11 件（1 件スキップ）、
  `tools/gdprobe/tests/test_compare.py` がすべて通過（メインでも discimg・gamecfg・lzma・test_expect を再実行して確認）。netgd の警告 0 件は変わらず。
- 比較（`stock_20261121_04_img3-7.txt`、`--only-common`）：230 → 200 行。SW_POLL・img=7 のすべて・REQ_SES の範囲外の差が消えた。残りは D2・D4・D8〜D11・D15（範囲外か、方針で残したもの）。
- 大きさ（text + data、383a2ccb → 805e98b4）：4E +664（165,236）、2E（`DEFMT_LOG=off`）+708（123,968）、2E `release-z` の `resident,sd-fast` +500（117,160 B、上限 122,368 B まで残り 5,208 B）。bss +24〜32 B。
- 実機で気をつける点：
  1. SECNR の OPEN（6）・BUSY（0）は推測。ゲーム中のボタンでの切替で、ゲームが「ふたが開いた」と受け取る可能性がある（純正も同じだと推測しているが未確認）。
  2. openMenu の `cdrom_reinit` は約 300 ms の NO_DISC のあと DISC_CHG を 1 回受ける見込み（待つ上限は 10 秒。INI で 5000 + 5000 を設定するとちょうど境目）。
  3. WSESSION_END と LOAD のあとにも約 300 ms の窓がある。netgd-recv の再試行は UA のときだけだが、0xE0 は窓の中でも答えるので問題ないと見ている。
  4. SRST と DEVICE RESET では窓を打ち切らない（ハードリセットのときだけ）。
  5. スタックの増え方は未測定（async の階層が 1 つ増えた）。
- 未確認（実機で確かめること）：キーの無い GDEMU.INI での純正の切替の時間、INI の値を変えたときに純正の時間が変わるか、存在しない番号への切替の純正の動き。
- 信頼度：純正の値は確認済み（実機）。NetGD の動きはホストのテストと予測だけで、実機では未確認。SECNR の値と、キーが無いときの時間は推測。
- 関連：R13、R18、R21、R119〜R125、`common/discimg/src/atapi.rs`・`slot.rs`、`common/gamecfg/src/ini.rs`、`mcu/netgd/src/ide.rs`・`gdi.rs`・`button.rs`、`tools/netgd-expect/README_ja.md`。

### R127 R125・R126 のコミット番号の訂正と、2E のスタックの静的な見積もり（Opus）

- **訂正**：R125・R126 のコミット `383a2ccb`・`805e98b4` は、main に入れる前に rebase したので、main では **`1e69ad2e`（R125）・`ad9f9720`（R126）**。内容は同じ。
- 担当：Opus（サブエージェントが解析、メインが確認）。問いは、R126 で非同期の呼び出しが 1 段深くなった 2E（ATSAM3U2E）で、スタックが足りるか。
- 方法：nightly の `-Z emit-stack-sizes` は使わず（何もインストールしていない）、DreamSDK の `arm-eabi-objdump -d -C` で逆アセンブルし、各関数のプロローグ（`push`・`stmdb`・`sub sp`）からフレームを、
  `bl` と関数の先頭への `b.w` から呼び出しグラフを作って、4 つのタスクの poll・`SysTick`・`ID_PIOA`・`main` から最も深い経路を探した。動的な SP 操作は 0 件。
  比較のため 43c5f4b4 も一時的な worktree で同じ条件でビルドした（削除済み）。スクリプトは scratchpad の `stack\stk.py`。
- 結論：
  - **確保されているスタック**（`link/sam3u2e.x`・`sam3u2e_resident.x`：`_stack_start = 0x20004000` から `__sheap` まで、ガード無し）：
    release `sam3u2e` = 7,536 B（43c5f4b4 は 7,568 B）、release-z `sam3u2e,resident,sd-fast` = 7,128 B（同 7,168 B）。PLAN A-1 の「約 7.9 KB」は今のビルドには当てはまらない。
  - **最も深い経路**：IDE タスクの poll（48）→ `sweep`（576）→ `handle_packet`（112）→ `load_media`（88）→ **`gdi::load_default`（1,208）→ `gdi::load_by_name`（3,280）** → `gdi::parse`（704）
    → … → `fat::lookup`（784）→ `memcpy`、合計 7,168 B（release）。release-z では 6,768 B（`load_by_name` 3,496 B）。
  - **R11・R21 の「約 5.1〜5.3 KB」は `load_by_name` から下だけを数えた値**で、呼び出し側の約 2 KB（poll から `load_media` まで約 0.8 KB、`load_default` 1.2 KB）が抜けていた。
    同じ数え方では 43c5f4b4 → ad9f9720 で release +8 B、release-z −8 B。IDE の経路全体では release +16 B、release-z −8 B。**R125・R126 の影響は数十バイト**。
  - 割り込みの優先度は設定されておらず（すべて 0）入れ子にならない。例外フレーム + 一番重いハンドラで最大 108 B と見積もった。
  - **余裕**：release = 7,536 −（24 + 7,168 + 108）= **236 B**、release-z の IDE 経路 = **228 B**、release-z の起動時の main タスク = 420 B。
  - async の状態はタスクの静的領域（`ide_task::POOL` 904 B、+8 B）に置かれ、スタックとは別。
- 危険：マウントの経路があと約 230 B 増えると、静的な最悪値が確保量を超える。ガードが無いので、溢れると下の静的データを黙って壊す
  （`__sheap` のすぐ下は `defmt_rtt::BUFFER` 1,024 B で、`DEFMT_LOG=off` では書かれないので実際にはクッションになっている可能性が高いが、偶然に頼っている）。
- 改善案（コードは未変更）：(1) `load_default` は「マウントする名前を決めて返す」だけにして、呼び出し側が `load_by_name` を 1 回だけ呼ぶ（約 1.2 KB 減）。
  (2) `load_by_name` の中の `VolumeManager` と 512 B のブロックキャッシュを static に移す（0.5〜1 KB 減）。
  (3) `DEFMT_LOG=off` のとき defmt の RTT バッファを小さくする、`flip-link` を有効にして溢れたら HardFault にする、起動時にスタックを既知の値で埋めて最高水位を BBA などで読めるようにする。
- 信頼度：フレームの値は高い（プロローグから正確に読める）。呼び出しグラフは同時に起きない組み合わせも含むので、多めに出る方向。間接呼び出し（waker の vtable、`walk_dyn` のクロージャ）は最大の経路を変えない。実機では未確認。
- 関連：R11、R21、R89、R125、R126、`docs/PLAN.md` A-1、`mcu/netgd/src/gdi.rs`。

### R128 純正は存在しない番号への切替でフォルダ 01 をマウントする（Opus）

- 担当：Opus。問いは R126 で未確認だった、番号のフォルダそのものが無いときの純正の `52 82 n`。
- 方法：試験用 SD（01〜09 だけ）のまま、gdprobe を `images=50,3`、`return_to=2` で実行。
- 記録：`docs/research/probe-records/stock_20261122_09_missing-folder.txt`（`abort=0`）。
- 結論：
  - `52 82 50` は即座に完了（0x50）し、ほかの切替と同じく 2/3A/00 → 約 174 ms 2/04/01 → 約 304 ms 6/28/00 → 約 335 ms READY。
  - マウントされたのは **フォルダ 01 の GDmenu**：GET_TOC = データトラック 2 本（FAD 150、FAD 12025）、REQ_SES_0 = `01 00 02 00 30 27`（セッション 2、lead-out FAD 12327）。
    `common/discimg` の `inspect` で `01\GDmenu_v0.6.cdi` を読むと、トラック 2 が LBA 11875（FAD 12025）・302 セクタで、lead-out は FAD 12327。完全に一致。
  - そのあとの `52 82 3` も普通に 03 をマウントした。
  - **NetGD（R126）は、番号のフォルダが無いと切替を断って前のイメージを残すので、純正と違う**。
  - SD の `GDEMU.ini` は `reset_goto = 1`。純正が「01」を選んだのか「reset_goto の番号」を選んだのかは、この試験では区別できない（R129 で切り分け）。
- 信頼度：確認済み（実機）。
- 関連：R13、R123（D12）、R126。

### R129 純正の切替の時間は GDEMU.ini の値に従う。存在しない番号は reset_goto によらず 01（Opus）

- 担当：Opus。問いは (1) R126 の推測「純正の 150 ms + 150 ms は GDEMU.ini の値」が正しいか、(2) R128 の「存在しない番号 → 01」が `reset_goto` によるものか。
- 方法：試験用 SD の `GDEMU.ini` を `open_time = 150`・`detect_time = 150`・`reset_goto = 1` から `open_time = 600`・`detect_time = 300`・`reset_goto = 2` に変えた
  （元の内容は `H:\NetGD_probe_sd\GDEMU.ini.orig` と scratchpad に控えた）。最初は `reset_goto = 3` にしたが、合成データの 03 は起動しないので、電源を入れてもメニューが出なかった（ユーザーの報告）。2 に直して実行。
  gdprobe は `images=3,50`、`return_to=2`。
- 記録：`docs/research/probe-records/stock_20261122_10_ini-600-300.txt`（`abort=0`）。
- 結論：
  - **切替の時間は INI に従う**：03・50・02 のどれも、2/3A/00 が約 608〜623 ms、2/04/01 から 6/28/00 までが約 300 ms、そこから約 30 ms で READY（03 = 932 ms、50 = 952 ms、02 = 956 ms）。
    150 / 150 のとき（R119）の約 150 ms・約 150 ms・約 30 ms と同じ形で、`open_time` と `detect_time` がそのまま 2 つの段階の長さになっている。
    **NetGD（R126）の「INI がキーを設定していればその値」は純正と同じ**。キーが無いときの純正の値はまだ未測定（R126 の 150 + 150 ms は推測のまま）。
  - **存在しない番号は 01**：`reset_goto = 2` でも、`52 82 50` のあとマウントされたのは GDmenu（GET_TOC = FAD 150・12025、REQ_SES_0 = `01 00 02 00 30 27`）。
    **純正は、番号のフォルダが無いとき `reset_goto` によらずフォルダ 01 をマウントする**。NetGD（R126）は断って前のイメージを残すので、直す（R132）。
  - `reset_goto` は、電源投入時・リセット時に読み込むフォルダ。起動できないフォルダにすると、メニューが出なくなる（注意）。
- 信頼度：確認済み（実機）。
- 関連：R119、R126、R128。

### R130 切替のあとのドライブの状態とヘッドの位置を純正に合わせた（D2・D4）（Opus）

- 担当：Opus（サブエージェントが実装、メインがホストテストを再実行して確認）。worktree `H:\OpenGDEmu-d2d4`、ブランチ `stock/state-head`、コミット `02732bad`（main ad9f9720 から）。
- 問い：純正の REQ_STAT・REQ_SES バイト 0・GET_SCD が示す状態と位置は、電源投入後と `52 82 n` の切替後でどう違い、NetGD をどう合わせるか（R123 の D2・D4）。
- 純正：電源投入後（切替の前）は待機 02・トラック 1・FAD 150（`stock_20261121_02.txt`・`05`・`06` の img=0：`02 2f 01 01 01 00 00 96`）。
  切替後は一時停止 01。位置は GDI がトラック 3 の先頭（03 は `01 8f 41 03 01 00 b0 5e`、GET_SCD_1 `00 15 00 0e 41 03 01 00 00 00 00 00 b0 5e`）、CDI がトラック 1・FAD 150（`01 2f 01 01 01 00 00 96`）。
  GET_SCD のオーディオ状態はどちらも 0x15。REQ_SES_0 のバイト 0 も切替後は 01。
- 結論（NetGD）：
  - `common/discimg`：`idle_fad(tracks, is_cdi, switched)` を追加（切替後の GDI = トラック番号 3 以上の最初のトラックの開始 + 150、それ以外と電源投入後 = 150）。
    `drive_status`・`req_stat_reply`・`head_position` が、CD-DA を始めていない状態の状態と位置を受け取るようにした（再生・一時停止・終了・エラーは従来どおり）。
  - `ide.rs`：`IDLE_STATUS`（初期値は待機）と `SWITCHED` を追加。`mounted()`（`mount_named`・`load_media`・ボタン・0xE0・reset_goto・取り出し後の再マウントが通る）で一時停止・切替済みにする。
    起動時のマウントは `mounted()` を通らないので待機のまま。DEVICE RESET の SECNR も同じ状態を使い、CD_SEEK の停止で待機に戻す。
  - 電源投入時に GDI を起動した場合は待機・トラック 1・FAD 150 とした（純正の電源投入時の記録は CDI だけ。推測）。CD_READ では状態も位置も変えない（読み出しのあと切替もリセットも挟まずに取った REQ_STAT が記録に無い）。
- テスト：純正のバイトを固定するテスト 3 件を追加。`common/discimg` 134 件・`test_expect.py` 11 件（1 件スキップ）がすべて通過（メインでも再実行）。
- 比較（`stock_20261121_04_img3-7.txt`、`--only-common`）：200 → 187 行（img=3 の REQ_STAT・REQ_SES_0/1・GET_SCD_0/1、img=4・5 の REQ_STAT・REQ_SES_0〜2 が消えた）。
- 大きさ（text + data、ad9f9720 → 02732bad）：4E +224、2E +228、2E `release-z` の `resident,sd-fast` +12（117,168 B、上限 122,368 B）。
- 残った差：Dreamcast をリセットして BIOS がもう一度起動したあとの img=0（記録 03・07・08）は、純正が一時停止・トラック 2・FAD 0x05017A（lead-out − 2）を返す。再現の規則が分からず、モデルにしていない。
  CD_SEEK の種類 4 では SECNR だけ一時停止にする（以前から）。
- 信頼度：記録との一致はホストで確認済み。電源投入時の GDI の扱いは推測。実機では未確認。
- 関連：R117、R119、R123、`common/discimg/src/atapi.rs`、`mcu/netgd/src/ide.rs`、`common/discimg/examples/expect.rs`。

### R131 イメージのマウント経路のスタックを約 2 KB 減らした（R127 の改善案）（Opus）

- 担当：Opus（サブエージェントが実装、メインが差分の範囲とホストテストを確認）。ブランチ `stock/state-head`、コミット `f590c961`（親 02732bad）。変更は `mcu/netgd/src/gdi.rs` だけ。
- 問い：R127 で余裕が約 230 B しかなかった 2E の最も深い経路を、動作を変えずに減らせるか。
- 結論：減らせた。原因は、インライン展開された `VolumeManager`（512 B のブロックキャッシュ込み、約 1 KB）が `load_default`・`load_by_name`・構文解析で重なっていたこと。
  - R127 の案 1（呼び出し側が `load_by_name` を 1 回だけ呼ぶ）は、「候補ごとに失敗したら次へ」（選択中の名前 → `GDEMU.CFG` の `image=` → フォルダ 01）と合わないので不採用。
  - `load_default`：候補名をインライン展開しない `default_name()`（`enum DefaultSource`）から、前の候補が失敗したときだけ順に取る。順序・フォールバック・警告は同じ。1,208 → 256 B。
  - `load_by_name`：`resolve_name()`（`walk_to`・`remember`・`locate_image`、戻るときにカードを閉じて `ImageRef` を返す）と `load_image` に分け、どちらも `#[inline(never)]`。
    3,280 → 336 B（代わりに `load_image` が 1,928 B、release-z 2,160 B だが、`resolve_name` とは重ならない）。`remember` が画像を探す前という順序とエラーの種類は同じ。
  - static 化（R127 の案 2）は不要だったので使っていない（BSS も増えない）。
- スタック（R127 と同じ静的解析、割り込み分 108 B を引いた余裕）：
  release 2E は最悪経路 7,168 → 5,200 B、余裕 260 → **2,228 B**。release-z 2E は 6,768 → 4,800 B、余裕 252 → **2,220 B**。起動時のマウントは 7,016 → 5,048 / 6,576 → 4,608 B。
- 大きさ（text + data）：4E −4、2E release +64、2E `release-z` の `resident,sd-fast` −20（117,148 B、上限 122,368 B）。
- テスト：`common/discimg` 134 件と `test_expect.py` がすべて通過（メインでも discimg を再実行）。netgd の警告 0 件は変わらず。
- 動作上の注意：`resolve_name` から戻るときにカードを閉じ、`load_image` が開き直す（読むだけなので書き込みは起きないはず）。ブロックキャッシュを引き継がないので、ルート付近の SD 読み出しが 1〜数ブロック増えるかもしれない。どちらも実機では未確認。
- 信頼度：ホストのビルドの静的解析とホストテストで確認。実機は未確認。間接呼び出しの先は数えていない。
- 関連：R11、R21、R127、`mcu/netgd/src/gdi.rs`（`load_default`・`default_name`・`load_by_name`・`resolve_name`・`load_image`）。

### R132 存在しない番号への切替でフォルダ 01 をマウントするようにした（R128・R129 の反映）（Opus）

- 担当：Opus（サブエージェントが実装、メインがホストテストを再実行して確認）。ブランチ `stock/state-head`、コミット `00e1ed5c`（親 f590c961）。
- 結論：
  - `discimg::slot::mount_target(n, exists)` を追加：n のフォルダがあればそれ、無くて 1〜999 なら 01、01 も無ければ「ディスクなし」、999 を超える欠番と 0 は従来どおり（0 = 既定に戻す、999 超 = 拒否）。
  - `ide.rs` の `stage_folder` が使う。01 があれば通常の `mount_named` に渡すので、切替の窓・切替後の一時停止・IP.BIN の先読みはほかの切替と同じ。01 が選択中のフォルダになり、ボタンと `52 81` は 01 から進む。
    01 にマウントできるイメージが無ければ D12 の「ディスクなし」。01 自体が無ければ「ディスクなし」にし、選択は切替の前のまま残す（ボタンが効かなくなるのを避ける NetGD の判断。純正は未調査）。
  - `52 82 0`・`52 81`・0xE0・`52 00` は変更なし。
- テスト：期待値を先に純正の動きに変更（`test_expect.py` の「欠番は断られる」を「欠番は 01 をマウント」に）。`common/discimg` 137 件（3 件追加）、`test_expect.py` 13 件（1 件スキップ）がすべて通過（メインでも再実行）。
- 比較（`stock_20261122_09_missing-folder.txt`、`images=3,50`、`--only-common`）：img=50 の差 50 → 39 行。SW_CMD・SW_POLL・TOC・REQ_SES・REQ_STAT・GET_SCD が一致。
  残りは CD_READ のバイト 1（純正は GDmenu の CDI に 0x24 を 5/64/00 で断り、0x10 で 2,352 B。D8〜D10）と GDEMU_VERSION（D15）で、以前からの差。
- 大きさ（text + data、f590c961 → 00e1ed5c）：4E +204、2E +80、2E `release-z` の `resident,sd-fast` +28（117,176 B、上限 122,368 B まで残り 5,192 B）。警告の増加なし。
- 未確認：01 が無いカード、999 を超える欠番の純正の動き（gdprobe の `images=` は 1〜999 だけ）。欠番のときはカードのルートを 2 回走査する。実機では未確認。
- 関連：R126、R128、R129、`common/discimg/src/slot.rs`、`mcu/netgd/src/ide.rs`、`tools/netgd-expect/expect_sd.py`。

### R133 スタックの最高水位を実機で読む仕組み（機能 `stack-hw`）と、R130〜R132 のコミット番号の訂正（Opus）

- **訂正**：R130〜R132 のコミットは main に入れる前に rebase したので、main では **`42028c49`（R130）・`d19775e2`（R131）・`b712f170`（R132）**。内容は同じ。
- 担当：Opus（サブエージェントが実装、メインが差分の範囲とホストテストを確認）。ブランチ `feat/stack-highwater`、コミット `a435063c`（親 b712f170）。
- 問い：PLAN A-1 のスタック（R127・R131 の静的な見積もり）を、動作を変えずに実機で測れるようにできるか。
- 結論：cargo の機能 **`stack-hw`（既定はオフ）** を付けたときだけ入る形にした。
  - **塗る**：`main` が `embassy_atsam3::init` の直後に、`__sheap` から「MSP − 32 B」までを `0xC5C5C5C5` で埋める（`mcu/netgd/src/stack.rs`）。
    `#[pre_init]` にしないのは、リセット直後の 4 MHz では数 ms かかる・`.data`/`.bss` の初期化前に Rust を動かすことになる・resident の VTOR を書く pre_init に触らずに済むため。
    `__sheap` より下（`defmt_rtt` のバッファを含む静的変数）は触らない。常駐部からの引き継ぎは GPBR とフラッシュだけで RAM には無い（`docs/design/resident-app-contract.md` 2-5 M1）。
  - **読む**：純粋なクレート `common/stackmark` が `__sheap` から上へ、最初に塗りの消えた語を探す（ホストテストあり）。
    `0xE0` のサブコマンド 9（STACK、読むだけ）が 20 バイト（`"NGSK"`、形式 1、reserved・used（最高水位）・unpainted、u32 LE）を返す。VERSION の機能バイトの bit 3（0x08）が STACK の有無。
    機能なしのファームではサブコマンド 9 は従来どおり ILLEGAL REQUEST。`0x52` と gdprobe は変えていない。
  - **画面と記録**：netgd-menu の情報画面（X）に `Stack  N of M bytes used`（超えたら ` OVERRUN`）、netgd-recv が `NGD:HELLO` のあとに `NGD:STACK ...` の 1 行。どちらも bit 3 があるときだけ送る。
- 既定にしない理由：2E `release-z` の `resident,sd-fast` はコードが +212 B でも、512 B 境界への詰め物でイメージが +512 B（117,676 B、上限 122,368 B）。2E release は +388 B。起動時に RAM へ書く約 0.2 ms も既定では足さない。
- 大きさ：機能なしは b712f170 と同じ（2E の 2 種は bin の SHA1 が一致。4E は大きさが同じで、defmt の番号の 67 B だけ違う）。機能ありは 4E +440、2E release +388、2E release-z +500、bss +8（2E release-z の reserved は 7,120 B）。
- テスト：`common/stackmark` 5 件、netgd-menu 77 件、netgd-recv 53 件、netgd-send 108 件がすべて通過（メインでも stackmark・netgd-menu・netgd-recv を再実行）。KOS 向けのビルドはしていない（C はホストの gcc でコンパイル）。CI のホストテストに stackmark を追加。
- 実機での読み方：`stack-hw` 付きでビルド・書き込み → 測りたい操作 → 電源を切らずにメニューに戻る → 一覧で X → `Stack` の行を読む（または netgd-recv のジョブの `NGD:STACK`）。比べる相手は R131 の 5,200 B（release）・4,800 B（release-z）。
- 限界：確保しただけで書かなかった分は数えない（小さめに出る）。塗らなかった上端は「使用済み」に数える。値はリセット以降の最大で、本体のリセットで MCU もリセットされるかは未確認。
- 信頼度：ホストテストとビルドは確認済み。実機での塗りと値は未検証。
- 関連：PLAN A-1、R127、R131、`mcu/netgd/src/stack.rs`・`main.rs`・`ide.rs`、`common/stackmark/`、`tools/netgd-menu/README_ja.md`（「スタックの最高水位を読む」）、`tools/netgd-recv/README_ja.md`。

### R134 CD_READ の期待セクタ種別の検査を純正に合わせた（D8・D9）（Opus）

- 担当：Opus（比較のサイドセッションが実装、メインがホストテストを再実行して確認）。ブランチ `atapi/cdread-type-check`、コミット `bd50704c`（main b712f170 から）。ユーザーの指示で取り込み。
- 純正（記録 10 回：dcload-ip、GDI 03・08、CDI 04・05、01 の GDmenu）：CD_READ のバイト 1 は、`0x20`（指定なし）はどちらも読める、`0x24`（Mode 1）は GDI で読めて CDI では CHECK 5/64/00、
  `0x28`（Mode 2 Form 1）は CDI で読めて GDI では CHECK 5/64/00、`0x10` はどちらも 2,352 B。7 セクタ読み（IP_READ）でも同じ。
- 結論：`common/discimg` に判定関数と ASC 64/00 を追加、`ide.rs` は読める範囲を調べる前に判定する。`expect.rs` も同じ判定を使う。古い「フラグを見ない」テストは純正の規則に書き換えた。
  音声トラックと、純正で試していない種別は、これまでどおり読ませる。`0x10` の 2,352 B（D10）は未対応のまま。
- 比較（`--only-common`）：記録 04 は 187 → 20 行、05 は 10 → 8 行、06 は 32 → 30 行。新しく増えた差は無い。残りは D10・D11・D14・D15。
- テスト：`common/discimg` 全通過（メインでも再実行）、`test_expect.py` 13 件（1 件スキップ）。
- 大きさ（text、main との差）：4E −44、2E +72、2E `release-z` の `resident,sd-fast` +516（上限まで残り 4,696 B。関数として説明できるのは `handle_packet` の +232 だけで、残りは配置の差と見ているが未確認）。
- 影響：CDI で Mode 1 を決め打ちして読むソフトは読めなくなる（純正と本物の GD-ROM ドライブも同じ）。R118・R123 で保留にしていたのは、今動くものを壊す方向の変更だったため。
  GD-ROM と CDI の両方で純正の規則を確かめた（R119）ので、取り込んだ。
- 信頼度：純正の値は確認済み（実機）。NetGD 側はホストテストと予測だけで、実機では未確認。
- 関連：R118、R119、R123、`common/discimg/src/atapi.rs`、`mcu/netgd/src/ide.rs`、`common/discimg/examples/expect.rs`。

### R135 GDEMU.ini に時間のキーが無いとき、純正の切替の窓は 500 + 250 ms（NetGD の既定値を修正）（Opus）

- 担当：Opus。問いは R126 で推測のままだった、`open_time`・`detect_time` が無いときの純正の切替の時間。
- 方法：試験用 SD の `GDEMU.ini` を `reset_goto = 2` の 1 行だけにして、gdprobe を `images=3,4`、`return_to=2`、`cdread_flags=0x20` で実行。
- 記録：`docs/research/probe-records/stock_20261122_11_ini-no-time-keys.txt`（`abort=0`）。
- 結論：03・04・02 のどれも、2/3A/00 が約 506〜515 ms、2/04/01 から 6/28/00 までが約 250 ms、そこから約 30 ms で READY（786〜795 ms）。
  **キーが無いときの純正の値は open 500 ms・detect 250 ms**（GDEMU の説明書きにある既定値と同じ）。150 / 150（R119）・600 / 300（R129）と合わせて、純正は INI の値を、無ければ 500 / 250 を使う。
- NetGD の修正：R126 で推測で入れた既定値 150 + 150 ms を **500 + 250 ms** に（`common/discimg/src/atapi.rs` の `SWAP_OPEN_MS`・`SWAP_DETECT_MS`）。
  テストの期待値を先に純正の値に変えて失敗を確かめてから直した（PLAN の決まり）。INI のキーがあればその値を使う動作（`Ini::swap_window_ms`）は変更なし。
  あわせて `examples/expect.rs` の冒頭のコメント（時間と、R134 で古くなった CD_READ の説明）を直した。
- テスト：`common/discimg` 138 件、`common/gamecfg` 31 件、`test_expect.py` 13 件（1 件スキップ）がすべて通過。2E `release-z` の `resident,sd-fast` のビルドが通る（警告の増加なし）。
- 影響：GDEMU.ini に時間のキーが無いカードでは、NetGD の切替のあとの NOT READY が約 300 ms から約 750 ms に延びる（純正と同じ）。
- 信頼度：純正の値は確認済み（実機）。NetGD はホストテストとビルドだけ。
- 関連：R119、R126、R129、`common/discimg/src/atapi.rs`、`common/discimg/tests/atapi.rs`、`common/gamecfg/src/ini.rs`。

### R136 AG16K の公開資料の追加調査（ピン探し用の回路のため）（Sonnet）

- 担当：Sonnet（ウェブ調査、ファイルは編集していない）。メインの Opus が要点を確認し、ピン探しの回路を作っているサブエージェントに渡した。
- 問い：`docs/design/g1-ag16k-port.md` 2-4 の方法 2（ピン探し用のビットストリーム）に必要な、AG16KDDF256 のボール表・PS のタイミング・内蔵の発振器・BSDL は公開されているか。
- 結論：
  - **ボール表**：公式のピン表（xls）は見つからない。AG16K データシート Rev1.0 の §4 は別冊を参照するだけで、別冊は未公開のよう。archive.org の `agm-micro` にも無い。
    使えるのは既存の `ag16k-config.md` §1-3 の DDF256 マニュアルの目読みだけ。EP4CE15F17 とのピン互換は DDF256 では成り立たない（既存の結論のまま）。データシートには専用クロックピン 15 本・I/O バンク 8 つ・PLL の入力は同じ辺のクロックピンから、とある。
  - **PS のタイミング**：新しい一次資料は無い（データシートは PS を「External Host with Flash Memory」とだけ書く）。MCU から AG10K/16K を PS で流した公開例も無い。
    rodinia の `chips.py` のデバイス ID は AG15K（AG16KSDE176）= `0x01500010`（DDF256 の項目は読めた範囲に無い）。
  - **内蔵の発振器**：Supra のプリミティブに **`alta_osc(input i_osc_enb, output o_osc)`** がある（rodinia が写した `alta_lib.v`・`alta_sim.v`。シミュレーションでは `i_osc_enb == 0` のときだけ発振）。
    周波数と DDF256 で配置できるかは不明。`alta_ufm` などにも `o_osc` があり、UFM のシミュレーションのコメントに 3.33〜5.56 MHz。af・Supra が組合せループ（リングオシレータ）をどう扱うかの記述は無い。
    AG1KLP では PLL が入力クロック無しで自走するという報告（二次資料）と、Yosys が暗黙に入れる `cycloneive_clkctrl` で af が失敗する（明示的にインスタンスすれば通る）という報告がある。
  - **BSDL**：見つからない。JTAG の IDCODE は `0x01500010` と推測（rodinia の device_id から。未確認）。Supra は USB-Blaster で JTAG に接続し、Query でデバイス ID を表示する。
  - **6.0 の FPGA のピン**：公開の解析は見つからない。AGM のビットストリームを読むツールは `pablomarx/rodinia`（`agm-unpack.py`・`agm-explain.py`・`ag16k-wires.json.gz`）。
- 法的な注意：rodinia は Supra の配布物（ALTAGATE の著作権表示あり）の写しを含むので、`alta_lib.v` などはリポジトリに写さない。archive.org の AGM 文書は「CONFIDENTIAL」の透かし付きの写しなので、ボール表の画像も置かない。純正のビットストリームの中身は使わない方針のまま。
- 根拠：AG16K データシート Rev1.0（https://atta.szlcsc.com/upload/public/pdf/source/20221219/4E43D2705D3E0F1E8FB34CC02EF10C1D.pdf）、
  https://archive.org/download/agm-micro/ （`Docs/MANUAL_Supra_6.2.pdf`）、https://github.com/pablomarx/rodinia（`bitstream/chips.py`、`supra/etc/arch/rodinia/alta_lib.v`・`alta_sim.v`、issue #1）、
  https://www.dreamcast-talk.com/forum/viewtopic.php?t=19395（本文は 403、検索の要約のみ）。調査メモは scratchpad の `ag16k-research.md`。
- 信頼度：プリミティブの宣言・デバイス ID は一次資料（Supra の配布物の写し）。周波数・配置可否・IDCODE は不明か推測。
- 関連：R32、R49、R66、R69、R99、R106、`docs/design/g1-ag16k-port.md`、`docs/research/ag16k-config.md`。

### R137 R125〜R135 のファーム変更の独立レビュー（Fable）

- 担当：Fable（読むだけ。テストとビルドで基準が緑であることを確認）。範囲は main の `43c5f4b4..ed4890b3`。
- 結論：**実機への書き込みを止めるべき重大な不具合は無い**。基板を壊す・ドライブが固まる経路、セカンダリ（書き込みディスク）の役割を変える変更も無い。
- 確かな指摘（中〜低）：
  1. 切替の窓の時間が TUR（CHECK、2/04/01）とホストの REQ_ERROR の間で切れると、`drive_not_ready` が REQ_ERROR の処理の先頭で窓を閉じて保留センスを 6/28/00 に上書きし、
     REQ_ERROR が 6/28/00 を返して UA も消す。err 0x60 の CHECK が一度も出ず、純正の並び（2/04/01 → CHECK 6/28/00 → READY）と違う（`ide.rs` の 1648 行付近・1461〜1466 行・1740〜1753 行・1844〜1855 行）。
     修正案：0x03・0x13 では `drive_not_ready` を呼ばない。
- 確度の低い懸念（実機で確かめたい）：
  - a. CHECK を 0x11（DRDY = 0）にしたので、FPGA がコマンド書き込みで DRDY を触らない作り（`fpga/amaranth/hw/ide_device.py` 612〜647 行）では、次の PACKET の CDB フェーズで status が 0x58 でなく 0x18 に見える。純正の CDB フェーズの値は未記録。
  - b. 窓の中の SECNR（OPEN 6・BUSY 0・NODISC 7）は純正のデータに基づかない唯一の部分（R126 で推測と明記）。
  - c. ホストが EJECT した状態で GET_TOC・REQ_STAT・REQ_SES・GET_SCD が NOT READY になった（純正のディスクなしとしては正しい）。
  - d. 起動時のマウントが失敗すると `EMPTY_SLOT` が立たず、ボタンが効かない（`52 81` は効く）。
  - e. `52 82 n` のルート走査中に SD の読み取りが失敗すると「フォルダ無し」扱いになり、01 も無い扱いでディスクを外す（以前は拒否して残していた）。
  - f. REQUEST SENSE（0x03）は ASC/ASCQ を返さない（R125 の既知の残り）。
  - g. CD_READ の種別検査はトラックのモードでなくイメージの種類（`is_cdi`）で判定する。Mode 1 のトラックを持つ CDI の純正の答えは未測定。
  - h. `52 00` のあとに REQ_ERROR が続くと、保留の UA が CHECK として報告されずに消える（以前からの動作）。
- 問題なしと確認したもの：窓の入口（`mounted()` が唯一）と出口、UA の二重発行が無いこと、CHECK のレジスタの経路、NOT READY の関門、空フォルダ・欠番の分岐、
  `resolve_name`/`load_image` の開閉、待機状態とヘッド位置と CD-DA の関係、CD_READ の種別検査（音声トラックは通す）、IDENTIFY PACKET、タスク間の並行性、INI の既定値。
- 対応：指摘 1・d・e と `ini.rs` の古いコメントを直す作業を Sonnet に依頼（ブランチ `fix/review-1`）。a・b・g は実機（gdprobe の記録の追加）で確かめる。
- 関連：R123〜R135、`mcu/netgd/src/ide.rs`、`common/discimg/src/atapi.rs`、`fpga/amaranth/hw/ide_device.py`。

### R138 CD_READ の 2,352 バイト読み（D10）を、既定は無効の機能 `raw-sectors` として入れた（Opus）

- 担当：Opus（比較のサイドセッションが自分の判断で調査・実装し、main に入れた。メインはテストと 2E のビルドを再実行して確認）。main のコミット `f8c891c1`（設計メモ）・`9f78590d`（本体）。ユーザーの指示で push に含めた。
- 設計メモ：`docs/research/d10-d14-raw-sectors.md`（純正が `0x10` の CD_READ と CDI の postgap のセクタ（D14）に返すバイト列を、生成コードと CRC で特定した内容と、実装の見積もり）。
- 結論：機能 `raw-sectors`（既定は無効）を付けると、CD_READ のデータ選択が `0x1_` のとき、2,352 B で保存されたトラックはそのまま、2,336 B で保存されたトラック（CDI の Mode 2）は同期・BCD の MSF・モード 2 のヘッダを前に生成して、
  1 セクタ 2,352 B を返す（純正が試したすべての GDI・CDI と同じ）。ほかのトラックは 2,048 B のまま。機能なしでは `out_sector_bytes()` が定数 2,048 で、動作は変わらない。
  `discimg::atapi` に `cd_read_wants_raw`・`raw_from_stored`・`mode2_raw_header`・`raw_sector`・`expand_raw_in_place`（ブロックの先頭のずれ 0〜511 と 1〜5 セクタで `raw_sector` と照合するテスト）。
  `ide.rs` は 1 回の要求について `RAW_OUT` を持ち、ステージング・キャッシュの確認・DMA・PIO のすべてで `out_sector_bytes()` を使う。2,352 B の読み出しの前後ではキャッシュを捨て、先読みはしない。
- 比較：`expect.rs --raw-sectors` の予測と純正の記録 04・05 で 20 / 8 → 17 / 6 行。予測に `compare --manifest` で 2,106 件一致、不一致 0。
- 大きさ（機能あり）：4E +1,664、2E +1,220、2E `release-z` の `resident,sd-fast` +1,008（上限まで残り 3,684 B）。機能なしは変化なし（メインで 2E `release-z` のビルドが通ることを確認）。
- テスト：`common/discimg` 全通過・`test_expect.py` 13 件（1 件スキップ）（メインで再実行）。
- 信頼度：ホストテストと予測で確認。実機では未確認。サイドセッションが、メインの依頼なしに main に取り込んだ（以後、取り込みはメインが確かめてから行う）。
- 関連：R123（D10・D14）、R134、`docs/research/d10-d14-raw-sectors.md`、`mcu/netgd/Cargo.toml`（`raw-sectors`）。

### R139 独立レビュー（R137）の指摘のうち小さな 4 件を直した（Opus）

- 担当：Opus（Sonnet のサブエージェントが実装、メインがホストテストを再実行して確認）。ブランチ `fix/review-1`（9f78590d から）、コミット `33440c5b`・`2b9bd8d6`・`209a5759`・`4bf3ba8e`・`10b2c33b`（コメントの番号を R139 に）。
- 結論：
  1. **センス読み出しと切替の窓**（R137 の指摘 1・h）：`discimg::atapi::advances_swap_window` が 0x03（REQUEST SENSE）・0x13（REQ_ERROR）で false を返し、この 2 つでは窓を進めも閉じもしない（次の通常コマンドで閉じ、そこで CHECK 6/28/00）。
     また `sense_read_clears_ua`：センス読み出しが UA を落とすのは、報告したキーが 6 のときだけ（`52 00` のあとの REQ_ERROR が、UA を報告しないまま消していた）。
  2. **起動時のマウント失敗**（R137 の d）：`ide_task` の初めに `EMPTY_SLOT = マウントされていない` を設定し、ボタンが D12 の空フォルダと同じく動く（成功時は変化なし）。
  3. **`52 82 n` の探索中の読み取りエラー**（R137 の e）：`slot::mount_target_checked`・`gdi::folder_lookup` で SD の読み取りエラーと「フォルダ無し」を区別し、エラーなら CHECK（ILLEGAL REQUEST）で断ってディスクを残す（R132 以前と同じ）。本当に無ければ R132 のまま 01。
  4. `common/gamecfg/src/ini.rs` の古い「150 + 150」のコメントを 500 + 250（R135）に。
- テスト：`tests/atapi.rs` 2 件・`tests/slot.rs` 1 件を実装の前に足して失敗を確認（PLAN の決まり）。`discimg` 全通過・`gamecfg` 31・`lzma` 5・`test_expect.py` 13（1 件スキップ）（メインで discimg・gamecfg・test_expect を再実行）。警告は変化なし。
- 大きさ（text + data、9f78590d → 4bf3ba8e）：4E +308、2E +196、2E `release-z` の `resident,sd-fast` +532（118,192 B、上限まで残り 4,176 B）。bss 変化なし。IDE タスクのスタックの静的な見積もり（release-z）4,800 → 4,808 B。
- 変更しなかったこと：`step_folder`（ボタンの前後）の読み取りエラーの扱い、`expect.rs`（時間を模擬しないので影響なし）、窓の時間。R137 の a・b・g は実機で確かめる（プローブの追加を準備中）。
- 信頼度：ホストテストとビルドで確認。`DeviceState`・`handle_packet` 自体はホストでテストできない（判定を `discimg` の純粋関数にしてテスト）。実機では未確認。
- 関連：R126、R132、R135、R137、`mcu/netgd/src/ide.rs`・`gdi.rs`、`common/discimg/src/atapi.rs`・`slot.rs`、`common/gamecfg/src/ini.rs`。

### R140 試験用 SD の GDEMU.ini を元に戻した（Opus）

- 試験用 SD の `GDEMU.ini` を、控えておいた元の内容（`open_time = 150`・`detect_time = 150`・`reset_goto = 1`、`H:\NetGD_probe_sd\GDEMU.ini.orig`）に戻し、MD5 で一致を確認した。
- gdprobe（`images=3`、`return_to=2`、読み出し試験なし）で、切替が 2/3A/00 → 約 149〜160 ms 2/04/01 → 約 299〜310 ms 6/28/00 → 約 330〜345 ms READY に戻ったことを確認
  （`docs/research/probe-records/stock_20261122_12_ini-restored.txt`、`abort=0`）。R119 と同じ。
- 関連：R119、R129、R135。

### R141 D10（`raw-sectors`）の独立レビューと、R137 の a・b・g を実機で確かめる準備（Fable・Sonnet）

- **D10 のレビュー**（Fable、読むだけ）：重大な不具合は無い。
  - 既定のビルド（機能なし）は命令列が完全一致ではないが動作は同じ（`.vector_table`・`.rodata`・`.data`・`.bss` は同一、`.text` は同じ大きさ。中身の違いは `handle_packet`（PIO の `out_sector_bytes()` が await をまたいで future に 2,048 を保存）と `gdi::reload_ini` のレジスタ割り当てだけ。DMA・キャッシュ判定・ステージング・先読みに命令の差は無い）。
  - 機能ありでも、バッファ溢れ・off-by-one・ヘッダの誤りは無い（`expand_raw_in_place` を先頭のずれ 0〜511 × 1〜7 セクタ × 2,352/2,336 の 5,666 通りで総当たり照合。同期 `00 FF×10 00`、BCD の MSF は `lba+150`、モード `02`）。
  - 指摘（低〜中）：音声トラックの扱いが文書（「音声は 2,048 B のまま」、`Cargo.toml`・設計メモ §3.1・R138）とコード（`ide.rs` 2026〜2027 行、格納 2,352 B なら音声でも 2,352 B の PCM を返す）で食い違う。純正の音声トラックへの `0x10` は未確認。
  - ほか：gdprobe の `0x10` は 1 セクタ・PIO だけなので、raw の複数チャンク・非マップ・DMA の経路は実機の試験でも通らない（G1 DMA は 32 B の倍数が前提なので、試すなら偶数セクタ）。raw 読みは IP.BIN の先読みキャッシュも捨てる（設計どおり）。
- **R137 の a・b・g の準備**（Sonnet、ブランチ `feat/gdprobe-more`、コミット `04607ca6`・`3520fd89`）：
  - gdprobe：PACKET の CDB フェーズで DRQ を見たときの ALTSTATUS を `cdbst=` に、SECNR（0xA05F708C、KOS `g1ata.c:90` の `G1_ATA_LBA_LOW`）を `secnr=` に記録（どちらも読むだけ）。
    `SW_POLL` は SECNR だけが変わっても 1 行書く。`compare.py` は 2 つの欄を両方の記録にあるときだけ比べる（古い記録との比較は差にならない）。この PC の DreamSDK でビルドが通る。ホストテスト 50,784 チェックと `test_compare.py` 24 件が通過。
  - mksdcard：`--set variants` にイメージ 10（データのセッションが **Mode 1**、2,352 B の生セクタで保存した CDI。レイアウトは 04 と同じ）を追加。`verify.py` で Mode 1 のセクタも照合する（`verify.py` に文字列の中に本物の改行が入った文法エラーがあり、メインが直した：`3520fd89`）。
    `H:\NetGD_probe_sd_v3` に生成し、08・09・10 がマニフェストと一致することをメインが確認。
  - 実行計画：`tools/gdprobe/README_ja.md` の「R137 の実行計画」と `gdprobe.cfg.r137.example`（`images=4,10,3,7`）。音声トラックへの `0x10`（上の D10 の指摘）も、合わせて確かめる項目に加える。
- 関連：R134、R137、R138、`tools/gdprobe/`、`tools/mksdcard/`。

### R142 ピン探し（設計 2-4 の方法 2）の PC 側の実装（Opus）

- 担当：Opus（サブエージェントが設計・実装）。ブランチ `feat/ag16k-pinfind`（ed4890b3 から）、コミット `5286e8e7`（本体）・`183fc8dc`（WSL で動かすファイルを LF に固定）。未取り込み。
- 問い：`docs/design/g1-ag16k-port.md` 2-4 の方法 2（ピン探し用のビットストリーム + MCU 側）を、基板と Supra なしでどこまで作って確かめられるか。
- 結論：
  - **FPGA 側（`fpga/pinfind/`）**：Amaranth の順序回路。ボールを 1 本ずつ、出力を切ったまま聞く（一度でも Low を読んだボールは飛ばす）→ そのボールだけ出力にして番号のフレームを送る → 次へ。`max_loops` 周（既定 8）で止まり、以後は全出力オフ。
    同時に出力になるのは常に 1 本以下（シミュレーションで毎クロック確認）。符号はパルス幅の比（SYNC = Low 8T + High 1T、ビット = Low 1T か 3T + High 1T）で、中身は番号とその反転。周波数が分からなくても読める。
    専用クロック入力 11 本は、同じフレームで周波数を数えて載せる。クロック源は `alta_osc`（既定、Low で有効と推定）・LUT 31 段のリング・ボール（DCLK を MCU が約 1 MHz で振る）の 3 通り（`src/pf_clock.v`）。
    出力は既定でオープンドレイン（Low だけ出す）。`gen.py` がボール一覧から Verilog・`.asf`・`.sdc` を `build/`（git の管理外）に出す。`build.sh` は WSL + Supra 用（`unshare -rn`、Supra のツリーには書かない）。
  - **ツール（`tools/pinfind/`）**：`mklist.py` が Supra の pinmap からボール名だけを書き出す（2 列目や機能名は写さない。コンフィグ・JTAG・NC・DDR は除く）。`pins2asf.py` が結果から本番の `.asf` を作る（1 本でも欠ければ `NETGD-PROVISIONAL-PINS` の印、終了コード 2）。
  - **復号器（`common/pinfind/`、no_std）**：パルス幅 → ビット → 番号、集計、周波数。sigrok の CSV を読む例もある（CN1 側をロジアナで読む用）。
  - **ファーム（`mcu/netgd` の機能 `pinfind`、既定は無効）**：PS の 5 本以外の全 PIO を入力 + プルアップにしてからビットストリームを流し、DWT で全ピンを見て RTT に出す。最後のフレームから 30 秒か開始から 300 秒で nCONFIG を Low にして止まる。SD への記録は無し。
  - CI に、pinfind のファームのビルド・`common/pinfind` のテスト・ツールのテスト・シミュレーションを追加。
- テスト：`fpga/pinfind/test_pinfind.py` 7 件、`common/pinfind` 17 件（Amaranth のシミュレーションの波形を Rust の復号器で読み、番号・周期・周波数が一致）、`tools/pinfind/tests` 12 件、既存の FPGA シミュレーションがすべて通過。
  既存のファームは 2E の 3 構成と board-v60 がバイト一致、4E は同じ大きさで defmt の番号の 16 B だけ違う。pinfind のビルドは仮のファイルを置いて text 35,384 B、スタックの余裕は約 6.6 KB。
- **未解決（実機の前に決めること）**：
  1. **Dreamcast に付けたまま動かしてはいけない**（Dreamcast が駆動している G1 の線と衝突する）。基板の外での給電方法が未確認で、いちばん大きな未解決点。SWD ヘッダの 1 番から入れる案はレギュレータへの逆流のおそれがあり勧めない。
  2. 専用ファームを SWD で書くと純正ファームは消える。**純正のバックアップ（R43 の fwbackup、ロックされていないこと）が取れてから**でないと実行しない。
  3. `alta_osc` の可否・極性・周波数、リングが af を通るか、DCLK を構成後に使えるか、Supra が `.asf` の I/O 属性を受け付けるかは不明（`build.sh` が切り分ける）。
  4. 残るリスク：VCC に直結されたボールや、相手が High を出している線には、聞いても区別できず Low を出してしまう（1 本あたり数十 ms × 最大 8 周）。
  5. PS の 5 本は v5.5 の仮の値、IRQn（PA22）・RSTn（PA30）の MCU ピンも v5.5 からの仮定。
- 信頼度：シミュレーションとホストテストは確認済み。ビットストリームは未作成、実機は未確認。
- 関連：R32、R49、R57、R106、R136、`docs/design/g1-ag16k-port.md` 2-4、`fpga/pinfind/README_ja.md`。

### R143 ピン探し（R142）の安全・設計レビュー：このままでは実機に流せない（Fable）

- 担当：Fable（読むだけ）。対象はブランチ `feat/ag16k-pinfind` の `5286e8e7`・`183fc8dc`。テスト：`test_pinfind.py` 7 件・`common/pinfind` 16 件（R142 の「17 件」は誤り）が通過。
- 結論：**実行前に必ず直す点が 4 つ**。FPGA の順序回路そのものに欠陥は無い。危ないのは Supra のフローが未検証なこと、設計に入っていないボールの扱い、ファームの例外経路。
  1. 【重大】`af` は位置指定が効かなかった I/O を警告（`placed at pin`）だけで任意の空きボールに置くが、`build.sh` は見ていない（本数の照合は通ってしまう）。`fixio.py` の `alta_io` の `.oe` の極性も未確認。
     逆なら走査対象の全ボールが同時に Low を出し続ける。→ 警告で失敗、配置後の「ポート → ボール」を `balls.txt` と完全照合、できればゲートレベルで「同時に Low は 1 本以下」を確認。
  2. 【重大】G1（DCLK）・G2（DATA0）・C1・D2 や DDR 側のボールが設計に入らず未使用ボールになる（Supra の未使用ピンの既定は不明。Quartus の古い既定は GND 出力）。
     一方 MCU は構成後も PA23 を Low で保持し、PA24 を約 1 MHz で振り続ける（`TOGGLE_DCLK = true`）。未使用が出力なら最長 300 秒衝突する（`ag16k-config.md` C9 の「G1・G2 は構成後に入力に固定」に反する）。
     → これらを明示的な入力（hold）として設計に入れる、`TOGGLE_DCLK` は `ball:` クロックのビルドだけ、構成後に PA23・PA24 を入力へ戻す、未使用ピンの扱いを報告で確定。
  3. 【高】走査中も MCU が LED の PB18 をプッシュプルで High にしている（`main.rs:172`）。「PS の 3 本以外は駆動しない」と矛盾。→ `pinfind` では LED を出力にしない。
  4. 【高】FPGA が動いたままになる経路：CONF_DONE が PB1 でないと MCU は失敗と見て無限に再ロードする／パニック・RTT の詰まり・デバッガ停止で nCONFIG が High のまま（`ball:` クロックではフレームの途中で 1 本が Low のまま固定）／
     `gen.py` が `--max-loops 0` を受け付ける／リセットのたびに 1 回分走る。→ ロード失敗で nCONFIG を Low にして停止、`early_done` は失敗、SysTick の割り込みから独立に期限で nCONFIG を Low、パニック時も Low、`max_loops` は 1〜8、再起動での再実行を GPBR で禁止。
  - ほか【中】：CN1 側の手順で `--drive pp` と `--no-skip-low` を併用すると保護が 2 つ外れる（→ 併用禁止、外付けプルアップで読む）。1 本あたりの Low の時間が長く絶対時間の上限が無い（→ `--no-probes --frames 2 --max-loops 1` から）。
    48 MHz のクロックボールは PCK0 を止めるので見つからない（→ 第 2 段で PCK0 だけ出す）。オープンドレインの立ち上がりに T ≥ 5 µs は足りない（→ 20 µs 以上）。
  - 【設計提案・要検証】逆向きの探し方：FPGA は全ボールを入力にして報告に既知の 1 本（G2 = DATA0 が候補）だけを使い、MCU が自分のピンを 1 本ずつ動かす。FPGA は未知のボールを 1 本も駆動しなくて済む。
- **基板外での給電（提案・要検証）**：純正ファームのまま、無通電で CN1 の電源ピン・U4・Q1・MCU の VDDIO・SWD の 1 番の導通と、各レールと GND の間の抵抗を記録 → Dreamcast に付けた状態でレールの電圧を基板上で測る →
  CN1 の電源ピンに同じ電圧を電流制限つきの電源（100〜150 mA から）で入れ、**純正ファームのまま基板外で起動する予行**をして基準値（電流・電圧）を取る。12 V に触れない、制限の無い電源を使わない、無通電で信号線をつながない、Dreamcast に付けたまま走らせない。
- **最初の実機実行の前に必須**：純正のバックアップ（fwbackup が UNLOCKED → dump → 照合。LOCKED なら実行しない）、PS の 5 本・MSEL1・LED のピンの導通確認、基板外の給電の予行、上の 1〜4 の修正、
  段階を踏む（段階 0 = 出力ゼロのビットストリームで CONF_DONE・電流・構成前後の MCU 全ピンの差分 → 段階 1 = 1〜数本だけ → 段階 2 = 全ボール → 段階 3 = クロック）、実行中は電流制限と記録、SD を抜く。
- 著作権：違反なし（Supra・rodinia のファイル、pinmap、ビットストリームは追跡対象に無い）。
- 対応：1〜6 の修正と段階 0 の用意を Opus に依頼（同じブランチ）。逆向きの方式は設計として検討する。
- 関連：R142、R43、R115・R116、`docs/research/ag16k-supra-build.md` §4-1、`docs/research/ag16k-config.md` C9。

### R144 純正の CDB フェーズの status・切替中の SECNR・Mode 1 の CDI・音声トラックの CD_READ（R137 a・b・g と R141 の実機確認）（Opus）

- 担当：Opus。gdprobe はブランチ `feat/gdprobe-more`（R141）のもの（`cdbst=`・`secnr=` を記録）。試験用 SD に `10`（Mode 1 の CDI）を追加し MD5 で照合。GDEMU.ini は元の内容（150 / 150 / 1）。
- 記録：`docs/research/probe-records/stock_20261122_13_r137-abg.txt`（`images=4,10,3,7`、`abort=0`）、`stock_20261122_14_audio-flags.txt`（`images=3`、`ip_fad=906`、`abort=0`）。
  `compare.py --manifest` で 03〜05（547 件）・08〜10（273 件）・音声（9 件）すべて一致、不一致 0。
- 結論：
  - **a（CHECK のあとの CDB フェーズの status）**：【R147 で訂正：DRDY は直前のコマンドの最終 status に従い、NetGD はすでに同じ。以下の「違う」は誤り】純正も、ディスクが無い（窓の中・空フォルダ）ときの CHECK のあとの PACKET では CDB フェーズの ALTSTATUS が **0x18**（DRDY = 0）。
    一方、ディスクがあるときの CHECK（CD_READ の種別違い 5/64/00）のあとは **0x58**（DRDY = 1）。CHECK の status 自体はどちらも 0x11。
    純正の DRDY は「CHECK を出したか」ではなく「準備できているか（ディスクがあるか）」に従う。**NetGD は FPGA が DRDY を最後に書いた値のまま残すので、ディスクがある CHECK のあとも 0x18 になるはず（違う）**。直すには FPGA（コマンド書き込みで DRDY を「準備できている」に合わせる）かファームの工夫が要る。
  - **b（切替中の SECNR）**：窓の中は `0x06`（OPEN、上位ニブル 0）→ 約 150 ms で `0x00`（BUSY）→ UA のころ `0xX1`（一時停止、上位はディスク形式：CDI 04 = 2、Mode 1 の CDI 10 = 1、GDI 03 = 8）。
    空フォルダは `0x06` → `0x00` → `0x07`（NO DISC）。**NetGD（R126）の推測の状態ニブル（6 → 0 → 1 / 7）は純正と同じ**。窓の中は上位ニブル（ディスク形式）が 0 になる点は要確認（NetGD の実装を見る）。
  - **g（Mode 1 の CDI）**：純正は 10 で `0x24`（Mode 1）を受け付け `0x28` を 5/64/00 で断る。REQ_STAT のディスク形式も **1（CD-ROM）**（`01 1f 01 01 01 00 00 96`）、SECNR も `0x11`。
    **純正は「CDI かどうか」ではなくトラックのモードで判定し、ディスク形式もトラックのモードから決める**。NetGD（R134 の種別検査は `is_cdi`、ディスク形式は CDI = 2）は違う。
  - **音声トラック（R141 の D10 の指摘）**：GDI 03 の音声（FAD 906）で、`0x20`（指定なし）と `0x10` は **2,352 B の PCM** をそのまま返し、`0x24`・`0x28` は 5/64/00。7 セクタの IP_READ（`0x20`）も 2,352 B × 7。
    **NetGD は音声トラックを種別検査で素通しにしていて（R134）、`0x20` で返す大きさも要確認**。
  - img=0（Dreamcast のリセットのあと）の REQ_STAT は再び `01 2f 41 02 01 05 01 7a`（一時停止・トラック 2・lead-out − 2、R130 の残りと同じ）。
- 対応：g と音声の規則（種別検査をトラックのモードで、音声は 0x24/0x28 を断り 0x20/0x10 で 2,352 B、CDI のディスク形式をトラックのモードで）を NetGD に入れる。a は FPGA の変更として扱う。
- 信頼度：確認済み（実機）。
- 関連：R126、R130、R134、R137、R138、R141。

### R145 ピン探しの安全対策（R143 への対応）（Opus）

- 担当：Opus（サブエージェントが実装）。ブランチ `feat/ag16k-pinfind`、コミット `a7a08fea`（PC 側）・`b0b3fcc3`（ファーム）。未取り込み。
- 結論：R143 の 1〜8 と 10 に対応した。
  - `build.sh`：`placed at pin` の警告で止まる。新しい `checkpnr.py` が Supra の出力から「ポート → 実際のボール」を読み、`balls.txt`（SHA-256 も確認）と完全照合する（読めなければ止まる。人が `--placement FILE --placement-regex` を指定）。
    未使用ピンは確かめる代わりに作らない方式：`$SUPRA_HOME` の pinmap をその場で読み、ユーザー I/O になりうるボールが 1 本でも設計に無ければ止まる（例外は `--unused-ok` で名指しし、報告に残す）。任意の `--gate-sim`（配置後のネットリストを Supra のモデルで Icarus Verilog、同時に Low は 1 本以下）。
  - hold：G1・G2・C1・D2・特別な機能のボール・DDR の F10・M10・L11・M11・クロックボールを入力として設計に入れる。入れないのは専用のコンフィグ・JTAG・H4 だけ。
  - `gen.py`：段階 0（出力ゼロ）、最初の実行向けの既定値（2 フレーム・1 周・プローブなし、T ≥ 20 µs）、`max_loops`・`frames` は 1〜8、pp と no-skip-low の併用を拒否。コアに駆動の見張り。
  - ファーム：ロードは 1 回だけで、失敗・`early_done`・量の不足なら nCONFIG を Low にして停止。SysTick の独立の期限（構成 30 秒、走査 120 + 3 秒、段階 0 は 20 + 3 秒）、panic・HardFault でも Low、500 ms 以上 Low の線で停止、GPBR3 でリセット後の 2 回目を禁止、
    構成後に PA23・PA24 を入力へ（DCLK は G1 クロックのときだけ）、LED を出力にしない、`embassy-atsam3` の `no-board-io` で SMC 等を設定しない、段階 0 で構成の前後と保持中の全ポートの PDSR を比べる。起動後 10 秒は FPGA を構成前のまま待つ（ロジアナを付ける時間）。
  - README：§0（必須の前提と段階 0 → 3）、§5（オープンドレイン + 飛ばす + 外付けプルアップ）、§9（逆向き方式の設計メモ）。
- テスト：`test_pinfind.py` 14・tools 14・`common/pinfind` 16・既存のシミュレーション 62 が通過。2E の 3 構成はロードされる中身が変更前と一致（4E は defmt の番号の 13 B だけ違い、ログを切ると一致）。
- 未解決：checkpnr が Supra の配置を読めるか（最初の実行で人が指定する見込み）、gatesim が動くか、hold の割り当てを Supra が受け付けるか、`alta_osc`・リングが通るか、`early_done` の扱い（今は安全側で毎回停止。判断が要る）、`ball:` クロックで MCU が止まる場合、§0 の前提（人の作業）。
- 信頼度：シミュレーション・ホストテスト・ビルドは確認済み。Supra の出力・実機は未確認。
- 関連：R142、R143、`fpga/pinfind/README_ja.md`。

### R146 4 ブランチの取り込み（R139・R141・R142〜R145）（Opus）

- 担当：Opus（メインセッション）。`fix/review-1`・`feat/gdprobe-more`・`feat/stack-highwater`・`feat/ag16k-pinfind` を main にマージ（`482f9198`、push 済み）。ユーザーの承認による。
- 衝突は `build.yaml`・`mcu/netgd/Cargo.toml`・`main.rs` の 3 つで、どれも stackmark と pinfind の両方を残すだけの和。
- 確認：ファームの 9 構成（4E、resident 4E・2E、release-z 2E、2E、2E / v60 + `stack-hw`、`pinfind`）のビルドと、ホストテスト（Rust 10 クレート、gdprobe、mksdcard、netgd-send、pinfind の tools とシミュレーション）がすべて通過。
- このPC（Windows）では `tools/netgd-recv`・`tools/netgd-menu` の C のテストが、マージ前の main でも同じように失敗する（recv は 52 件中 43 件、menu は 11 件のエラー。`job.ngj missing`・一時ファイルの作成失敗など、Windows の環境によるもの）。CI（Linux）で確認する。マージで増えた失敗はない（recv の追加テスト 1 件は同じ理由で失敗）。
- `fix/raw-audio`（e2cad6af）は R144 の純正の結果（音声で 0x20・0x10 は 2,352 B）と逆なので取り込まない。ブランチと作業フォルダの削除はユーザーの確認待ち。
- 信頼度：ビルド・ホストテストは確認済み。実機は未確認。
- 関連：R139、R141〜R145。

### R147 CHECK のあとの CDB フェーズの DRDY（R144 a の訂正）（Sonnet 調査・Fable レビュー・Opus 確認）

- **訂正：R144 a の読みは誤り**。純正の CDB フェーズの DRDY は「ディスクがあるか」ではなく、**直前のコマンドの最終 status の DRDY** に従う（成功 0x50 → 次の CDB フェーズは 0x58、CHECK 0x11 → 0x18）。
  記録の各行の `cdbst=` はそのコマンド自身の CDB フェーズで、前のコマンドの結果を表す。R144 が「ディスクがある CHECK のあとは 0x58」としたのは CDREAD_FLAGS の行で、`tools/gdprobe/probes.c` が CHECK のたびに隠れた REQ_ERROR（成功 0x50）を送るため、次の行は 0x50 のあとだった。
- 確認：隠れたコマンドのない隣り合う行の組を数えると、記録 13 は 0x50 → 0x58 が 67 組・0x11 → 0x18 が 23 組、記録 14 は 27 組・3 組で、例外 0（ディスクあり・なし・窓の中とも）。
- **NetGD の main はすでに純正と同じ**：FPGA は MCU が最後に書いた status の DRDY を保ち、PACKET でも変えない（`fpga/amaranth/hw/ide_device.py` の `drdy`）。ただし NetGD の実機では `cdbst` を測っていない（コードを読んだ結論）。
- ブランチ `exp/drdy`（`449fd690` FPGA・`20e83a4b` ファーム、Sonnet）は誤った前提で作られ、取り込むとディスクありの CHECK のあとが 0x58、ディスクなしの成功のあとが 0x18 になって純正から離れる（Fable のレビュー）。**取り込まない**。削除はユーザーの確認待ち。
- 調べて分かったこと（参考）：MCU が PACKET を知るのは約 100 µs 後で、CDB フェーズの status をファームから合わせることはできない。
- 信頼度：純正の規則は確認済み（実機の記録）。NetGD 側はコードから。gdprobe で NetGD の `cdbst` を記録 13 と比べれば確定する。
- 関連：R144、`docs/research/probe-records/stock_20261122_13_r137-abg.txt`。

### R148 R144 の純正の規則を NetGD に入れた（種別検査・音声トラック・ディスク形式・切替中の SECNR をトラックのモードで）（Opus）

- 担当：Opus（サブエージェントが実装）。ブランチ `feat/stock-rules`、コミット `b664e29b`（R146 のマージの先端から）。未取り込み。
- トラックにモード（音声・Mode 1・Mode 2）を持たせた：CDI は DiscJuggler のトラックモード（記述子の +16）、GDI のデータは Mode 1（2,336 B は Mode 2）。ファームはセッション番号のバイトの bit 7 に入れ、`Track` は 32 B のまま（別のバイトにすると bss +392 B）。
- CD_READ の種別検査はトラックのモードで判定（`cd_read_type_mismatch(flags, mode)`）：Mode 1 は 0x28、Mode 2 は 0x24、音声は 0x24・0x28 を 5/64/00。0x20・0x10 は常に読める。
- 音声トラックは 0x20・0x10 で 2,352 B の PCM をそのまま返す（複数セクタも）。従来は 2,048 B（PCM の +16 から）だった。2,352 B の経路を `raw-sectors` の外に出し、この機能はデータトラックの 0x10 だけを足すものにした。
- ディスク形式：GDI 0x80、CDI は Mode 2 があれば 0x20、Mode 1 だけなら 0x10（純正の `01 1f ...`、SECNR 0x11）、音声だけなら 0x00（未測定、Flycast と同じ）。
- 切替中の SECNR：0x06 → 0x00 は上位ニブル 0、窓のあとに 0xX1（空フォルダは 0x07）。従来は窓の中もディスク形式を出し（CDI で 0x26 など）、空フォルダは 0x87 だった。窓の中とイメージなしの `disc_format()` は 0。
- テスト：`common/discimg` 149 件（6 件追加）、`test_expect.py` 13 件（1 件スキップ）。expect の予測と記録 13・14 の差は GDEMU_VERSION（D15）だけ。
- 大きさ：2E `release-z` の `resident,sd-fast` は +12 B（118,188 B、上限 122,368 B まで残り 4,180 B）、bss は変わらず。
- 実機での注意：音声トラックの CD_READ は既定のビルドでも 2,352 B の経路を通る（実機未確認。奇数セクタの DMA は 32 B の倍数にならない）。イメージなしで起動したときの SECNR は 0x82 → 0x02。記述子にモードが無い CDI の扱いは推測。
- 信頼度：ホストのテストと予測は確認済み。実機では未確認。
- レビュー（Fable）への対応（コミット `f0dccd36`、親 `b664e29b`）：
  - 起動時と DEVICE RESET で、イメージが無い（`media_present` が偽）ときの SECNR を `0x07`（NO DISC、形式 0）にした（`rest_secnr`）。窓の中の DEVICE RESET・バスリセットは窓の値（`window_secnr`）を書く。
  - 切替の窓の SECNR を時間で進めるようにした：`ide_task` の待ちを「PIOA の信号か、次の段階の境目のタイマー」にし、タイマーでは SECNR だけを書き直す（`0x06` → open_time で `0x00` → 合計で `0xX1`／`0x07`）。窓を閉じることと UA は従来どおり次のコマンドで。
  - CDI の記述子のモードが 1・2 なら、control が音声でもそのモードのデータとする（Flycast と同じ）。control の値は TOC のためにそのまま残す。
  - 2,352 B で格納されていない音声トラックの CD_READ は ILLEGAL REQUEST（`cd_read_unservable`）。
  - `Track` の ctrl（下位 4 ビット）とモード（ビット 4〜5）を 1 バイトに詰めた（`discimg::pack_ctrl_mode`、ホストテストあり）。32 B であることをコンパイル時に確認。
  - テスト：`common/discimg` 151 件、`test_expect.py` 13 件（1 件スキップ）。2E `release-z` の `resident,sd-fast` は 118,684 B（`b664e29b` から +508 B、上限 122,368 B まで残り 3,684 B）、bss は −4 B。
- 関連：R126、R130、R134、R141、R144。

### R149 C のツールのテストが Windows で通らなかった原因（R146 の訂正）（Fable）

- 担当：Fable（サブエージェント）。ブランチ `fix/ctests-windows`、コミット `b54c43f0`。未取り込み。
- 原因：このPCで最初に見つかる `python` は DreamSDK の MSYS 版（`C:\DreamSDK\usr\bin\python.exe`、`sys.platform=cygwin`）。テストは `os.name` で `.exe` の有無を決め、`tempfile` の POSIX パス（`/tmp/...`）を渡すので、MSYS が `/` で始まる引数を書き換えて、ネイティブの gcc（scoop の mingw-w64）で作ったテストが別の場所を見る（例：`/02/DISC.GDI` → `C:/DreamSDK/02/DISC.GDI`）。C のコードとテストの不具合ではない。
- 確認：ネイティブの Python（`py -3`）なら、変更なしの main で netgd-recv 53 件・netgd-menu 77 件がすべて通過（メインでも recv を再確認、exit 0）。
- R146 の「Windows の環境で失敗する」は、この原因だった。
- 修正：両方の `run_tests.py` の先頭に、MSYS / Cygwin の Python とネイティブの gcc の組み合わせなら「`py -3` で実行して」と言って止まる確認を入れた（Linux とネイティブの Python は素通り）。README の `python …` はそのまま。
- 信頼度：確認済み（このPC）。Linux での無害さは CI で確認する。
- 関連：R146。

### R150 バスリセットのあとの SECNR と、SECNR の読み→書きの隙間（R148 のレビューへの対応）（Opus）

- 担当：Opus（サブエージェント）。ブランチ `fix/secnr-reset`、コミット `237130e6`（親は main `6ef6dc87`）。未取り込み。指摘は比較のセッションのレビュー。
- バスリセット（SRST・RSTn）のあと、窓の外でディスクが無いと SECNR が 0x01（EJECT のあとは旧形式|1）になっていた。純正どおり 0x07 にした（`discimg::atapi::reset_secnr`、`reassert_disc_format` と `cancel_swap_window` の両方）。
- ディスクがあるときは従来どおり、形式と FPGA が戻した状態ニブル 1 のまま。`rest_secnr` を使うと初回切替前が待機 0x_2 になるが、純正の記録が無いので使わない。
- FPGA のスロット 6 は MCU の書き込みで lba_low と lba_mid を同時に書く（バイトレーン無し、`ide_device.py` 1121〜1131 行）。lba_mid はホストが BYCTLLO を書く場所なので、`set_secnr` の読み→書きの間にホストが書くと値が消える（窓のタイマーと CDDA 終了の経路。コマンド処理中はホストが BSY を待つので起きない）。
- ファームでは読み→書きを割り込み禁止にして隙間を縮め、タイマーは値が同じなら書かない。完全に直すには FPGA でスロット 6 を下位バイトだけ書けるようにする（スロット 5 と同じ）。次のビットストリームの候補。
- テスト：`common/discimg` 152 件（1 件追加）、4 種類のビルドが成功、2E release-z は +8 B。実機は未確認。
- 関連：R126、R148。

### R151 D14：Form 2 のセクタを 2,324 B で返す（feature `form2-sectors`）（比較のセッション）

- ブランチ `feat/form2-sectors`、コミット `b7b3b578`（main `6ef6dc87` に付け直し済み）。既定はオフ。
- CDI の 2,336 B 格納の Mode 2 トラックで、読むセクタがすべて Form 2（サブモードのビット 5）のとき、`0x20` で 2,324 B／セクタ（[8:2332]）を返す。Form 1 と混ざる読み、`0x28`、5 セクタを超える読み、写像の無いトラックは 2,048 B のまま。サブヘッダは SD の 1 回の読みで確かめ、Form 2 でなければ通常の 2,048 B のキャッシュに使う。
- 予測と記録 05（イメージ 4・5）の差は 8 → 4 件（SEEK_01 の 2,324 B と crc が一致）。大きさは既定で 2E release-z +20 B、機能ありで +532 B。テストは先に書いて失敗を確認。実機は未確認。

### R152 純正の記録と予測の全面比較・容量の内訳・予測の cdbst / secnr（比較のセッション）

- ブランチ `docs/stock-audit`（`52910f65`、`docs/research/stock-diff-audit.md`）、`feat/expect-cdbst-secnr`（`f4498c46`、`docs/stock-audit` から）。
- 比べられる記録 13 本の差は、既定で 102 件、`--raw-sectors --form2-sectors` で 75 件。残りは D15（GDEMU_VERSION、27 行、承知の上）と D11（純正がマウントしない形の GDI、48 行。今の動きのままにする）。予測の誤りは無し。
- 予測が `cdbst=`（直前が CHECK なら 0x18、そうでなければ 0x58。gdprobe が記録しない CHECK のあとの REQ_ERROR と切替のポーリングも数える）と `secnr=` を出すようにした。記録 13・14 ですべて一致。
- 容量（2E release-z の `resident,sd-fast`）：c8d2cb02 から +1,016 B で、上限まで残り 3,664 B。大きいのは `handle_packet` 8,956 B・`main_task` 4,332 B・`sweep` 4,236 B・`load_image` 3,764 B・embedded_sdmmc 計 6.6 KB。削る案は A（めったに使わないコマンドを分ける）・B（`sweep` の展開）・C（名前をバイトで扱う）・D（embedded_sdmmc の置き換え）。LZMA の辞書を大きくしても −99〜−554 B だけ。

### R153 名前と INI の読み取りをバイトで扱う（R152 の案 C）（比較のセッション実装、Fable レビュー）

- ブランチ `perf/name-bytes`、コミット `e5732b62`（main `6ef6dc87` から）。`common/gamecfg`・`common/fatmap`（lfn.rs）・`mcu/netgd/src/gdi.rs` を `&str` からバイト列に。2E の像から `core::str`・`core::char` が消えた。
- 大きさ（text）：4E −2,428 B、2E −3,380 B、2E release-z の `resident,sd-fast` −2,556 B（115,968 B、上限まで残り 6,240 B）、v60 −3,384 B。
- テスト：gamecfg 37・fatmap 52・discimg 151 件。旧（`&str`）と新の結果を約 40 万の生成入力（CRLF・タブ・VT/FF・BOM・Shift-JIS・不正な UTF-8・長すぎる値）で比べるテストを追加。
- Fable のレビュー：挙動の違い・新しい panic の経路は無し。`widened_eq` は 300 万組で旧と一致（使い捨てのテスト）。指摘は小さなものだけ（`widened_eq`・`mounted_number` のホストテストが無い、`fit` の unsafe の前提、gdi.rs の `trim_ascii` が VT を空白として扱わない（main と同じ））。実機は未確認。

### R154 gdprobe：切替中のレジスタのポーリング・窓の中の DEVICE RESET・音声だけの CDI（R148 の確認用）（Opus）

- ブランチ `feat/gdprobe-r148`、コミット `d615fa63`。記録の形式は 2。
- `swap_regpoll`：`52 82 n` のあと 1,000 ms はパケットを送らず、ALTSTATUS・ERROR・割り込み理由・SECNR を 2 ms ごとに読み、変化を `SW_REGPOLL` で記録（STATUS は読まないので割り込みは消さない）。
- `swap_devreset`：同じイメージにもう一度切り替え、50 ms・200 ms 後に ATA DEVICE RESET（0x08）を送り、署名レジスタと SECNR を `R<ms>_SW_DEVRESET` で記録。ATA のホワイトリストを新設（0x08 と 0xA1 だけ）。
- mksdcard のイメージ 11：音声トラック 2 本だけの CDI（データトラック無し）。データトラックが無いときは `NO_DATA_TRACK` を記録して IP_READ・速度・シークを飛ばす。
- テスト：gdprobe 57,668 件、compare 27 件、mksdcard 23 件。カードの中身は `H:\NetGD_probe_sd_v4`（`11/disc.cdi` MD5 `34f49644…`）。実機の実行はこれから。

### R155 純正の切替中のレジスタ（コマンドなし）・窓の中の DEVICE RESET・音声だけの CDI（R154 の実機実行）（Opus）

- 担当：Opus。gdprobe は `feat/gdprobe-r148`（`d615fa63`）。試験用 SD に `11`（音声だけの CDI）を追加し MD5 で照合。GDEMU.ini は元の内容（150 / 150 / 1）。
- 記録：`docs/research/probe-records/stock_20261122_15_regpoll-devreset.txt`（`images=4,3,11,7`、`return_to=2`、`abort=0`）。`compare.py --manifest` は 547 件一致・不一致 0（イメージ 11 の 2 件も一致）。
- 結論：
  - **コマンドを送らなくても SECNR は時間で進む**：`52 82 n` の約 3 ms 後に `0x06`、150 ms で `0x00`、300 ms で形式|1（CDI 04 = 0x21、GDI 03 = 0x81、音声だけの CDI 11 = 0x11）、空フォルダ 07 は `0x07`。**NetGD の `f0dccd36`（窓のタイマーで SECNR を書き直す）は純正と同じ**。ALTSTATUS はこの間 0x50 のまま。
  - **窓の中の DEVICE RESET（0x08）**：50 ms・200 ms のどちらでも、BSY のあと ALTSTATUS `0x00`・ERROR `0x01`・割り込み理由 `0x01`・署名 `14 EB`、SECNR は**すぐに `0x00`**、そして**リセットから約 150 ms 後に最終の値**（0x21 / 0x81 / 0x11 / 0x07）。最初の `0x06` には戻らない。ALTSTATUS は次のコマンドまで `0x00`（DRDY = 0）。そのあとの TUR の流れ・UA・REQ_STAT は通常と同じ。
    **NetGD（R148 の `f0dccd36`）は窓の中の DEVICE RESET で窓の値（open 中なら `0x06`）を書くだけで、純正と違う**。純正は「DEVICE RESET で検出の段階（150 ms、`detect_time` と同じ値なので区別はまだ）をやり直す」ように見える。
  - **音声だけの CDI のディスク形式は 1（CD-ROM）**：SECNR `0x11`、REQ_STAT `01 1f 01 01 01 00 00 96`。**NetGD（R148）の推測 0（Flycast）は違う**。TOC は音声 2 本、REQ_SES(2) は CHECK。音声の CD_READ は R148 の規則どおり（0x20・0x10 は 2,352 B、0x24・0x28 は 5/64/00）。
- 対応：音声だけの CDI のディスク形式を 1 にする。窓の中の DEVICE RESET は SECNR `0x00` にして、リセットから検出の段階をやり直す。DEVICE RESET のあとの ALTSTATUS（0x00）を NetGD でも確かめる。
- 信頼度：確認済み（実機）。
- 関連：R126、R144、R148、R154。

### R156 R155 の対応（音声だけの CDI = 1、窓の中の DEVICE RESET）と、記録 15 で見つかった新しい差（Opus）

- 担当：Opus（サブエージェントが実装）。ブランチ `fix/r155-stock`、コミット `1bc2080a`（main `34b57a74` から）。未取り込み。Fable のレビュー中（2026-11-22 時点で結果待ち）。
- 音声だけの CDI のディスク形式を 1（CD-ROM、SECNR 0x11）に。規則は GDI 0x80・Mode 2 があれば 0x20・それ以外の CDI は 0x10（`discimg::atapi::disc_format`）。
- 窓の中の DEVICE RESET は、リセットの時刻から窓を検出の段階でやり直す（`swap_window_after_device_reset`、長さは detect_time。純正の 150 ms は open_time とも同じ値なので未区別）。SECNR はすぐ 0x00、detect_time 後に最終の値。完了は ALTSTATUS 0x00・ERROR 0x01（従来 0x50・0x00）。窓の外は変えていない。
- 予測に `--regpoll`・`--devreset`・NO_DATA_TRACK を追加。記録 15 との差は REGPOLL 以外は D10・D14・D15 だけ。SW_DEVRESET と、そのあとの REQ_STAT・REQ_ERROR はすべて一致。
- **新しい差（未対応）**：
  - 純正は成功したコマンドのあとも ERROR レジスタに前の sense key（0x50 など）を残す（NetGD は 0x00）。gdprobe のコマンドごとの行では err=0x00 と出るので、レジスタのポーリングでだけ見える。
  - 同じイメージへの再切替（R50・R200 の回）では、純正の SECNR は 0x06・0x00 を出さず前の値のまま（UA は来る）。NetGD はマウントのたびに窓を開く。純正の DEVICE RESET が窓に関係なく検出をやり直す可能性もある（データ無し）。
- 次の実機の回の準備：`H:\OpenGDEmu-probe4\tools\gdprobe\host\gdprobe.cfg` を seq 16（`images=4,3,7`、`swap_devreset_ms=600,2000`）にした。窓の外での同じイメージへの再切替（リセットまでの 600 ms・2,000 ms に窓を開くか）と、窓の外の DEVICE RESET と、ERROR レジスタを確かめる。
- テスト：discimg 155 件、test_expect 15 件（1 件スキップ）、4 種類のビルドが成功。2E release-z はコード +56 B（パディングの境目を越えて像は +504 B、上限まで残り 5,888 B）。実機は未確認。
- 関連：R148、R150、R155。

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
| 2026-11-20 | 対策 B：D1 復旧は SD だけ／D2 常駐部 8 KB（FPGA ローダーは入れない）／D3 当面は SWD で導入（移行イメージは作らない）／D4 常駐部が自己ロック（実機確認までは開発用ビルドでロックしない）／D5 SD 更新はアプリ無効・ボタン・アプリの要求のときだけ／D6 ファイル名 `NETGD.OGF`・`NETGD.BIT`、LED の表示は 6.0 の LED のピンが分かるまで保留／D7 S0〜S2 は今すぐ、S3 以降は実機の後 | `docs/design/fw-bootloader.md` §6-2（R54）。8 KB は、未確定の AG16K ローダーをロック領域に凍結しないため | ユーザー（Opus の推奨を承認） |
| 2026-11-20 | 上流から引き継いだ ATAPI 応答の疑い（R11 の D）は、実機と比べるまで挙動を変えない | 実機の純正 GDEMU が仕様や Flycast と違う応答をしている可能性がある（コード内コメント） | Opus |
| 2026-11-20 | GitHub の公開先は iwa-kasoutuuuuuka/NetGD（公開）、上流は `upstream` リモートに残す | ユーザーの指示 | ユーザー |
| 2026-11-20 | 番号フォルダ方式を採用し、設定が無ければフォルダ 01 をマウント。`0x52` で番号指定・次／前に対応 | 本家と GDmenu / openMenu との互換（R13・R14） | ユーザー（G3 を選択）、Opus 設計 |
| 2026-11-20 | `0x52 82` で番号が見つからないときは、今のディスクを外さずにエラーを返す | openMenu はエラーを見ずに READY を待つため | Opus |
| 2026-11-20 | プローブツールは別 PC の DreamSDK でビルドし、KOS 2.3.0（ユーザーのフォーク）に合わせる。純正には読み取り系コマンドだけを送る | このPCに KOS が無い。純正ファームを壊さないため（R19） | ユーザー（ビルド環境）、Opus（安全方針） |
| 2026-11-20 | F5（リセット後の再マウント）は RSTn の立ち下がりカウンタで判定し、SRST では動かさない | SRST は KOS の `g1ata` 初期化で出て両デバイスに効くため、再マウントすると書き込みが全部拒否される（R15・R16） | Opus |
| 2026-11-20 | `netgd-recv` の修正（R48 の 11 項目・CI の失敗・テストが固まる件）を途中で Sonnet から Fable に引き継ぐ。Sonnet の未コミットの変更は残し、Fable が取捨する | 固まった原因の究明はデバッグで Fable の担当。中核に無限ループがあれば実機でも固まるため | ユーザー |
| 2026-11-20 | SLB 0 と CLB 0 が1ビット違いである危険は受け入れる。常駐部はロックの後に GLB で読み直し、外れていれば次の起動で再ロックする | SLB の引数はどちらの読み方でも領域0を指す 0 しか安全でない。CLB 0 はロックが外れるだけで壊れない（R61） | Opus |
| 2026-11-20 | `netgd-recv` のホストテストは quick を既定、CI は full。作業フォルダはシステムの一時フォルダ | USB HDD 上での実行が遅さの原因だった。CI は `CI=true` で自動的に full（R62） | Fable 提案、Opus 承認 |
| 2026-11-20 | 常駐部の FAT リーダーは、有効な FAT の指定（BPB 0x28）と MBR の4項目すべてに対応する | ロック後は SWD でしか直せない。実在しうるカードを拒否する理由が無く、数行で済む（R67） | Opus |
| 2026-11-20 | G3-4 の高速化（`fast-complete`・`sd-multiblock`・`sd-highspeed`）は既定ではオフの feature にする | 実機で測るまで既定の動作を変えない。完了のタイミングは実機で壊れた前歴がある（R72） | Opus |
| 2026-11-20 | クレート・バイナリ・フォルダを `netgd` に改名。IDENTIFY の機種名など、ファームが外に見せる名前は変えない | 既存のツールや純正との比較が機種名に依存しうる。更新ツールは両方の名前を受け付ける（R76） | Opus |
| 2026-11-20 | 6.0 のビットストリームの置き場所（SD の `NETGD.BIT` か埋め込みか）は、Supra で実際のサイズを測ってから決める。それまで SD 方式は実装しない | AGM 独自の圧縮で埋め込める大きさになる可能性がある（R66・R69） | ユーザー |
| 2026-11-20 | 常駐部とアプリの取り決め E1〜E9 を推奨どおり採用（E1 巻き戻しのみ、E2 `handoff()`、E3 GPBR1 と情報ブロックを世代 1 に、E4 V3、E5 `"NGAX"` を今追加、E6 ピン不明のうちは 6.0 の常駐部をロックしない、E7 `RSTC_CR` に統一（S5 で）、E8 LED は D6 と一緒に、E9 `resident` から形式2の更新プログラムを外す） | `docs/design/resident-app-contract.md` §7（R79） | ユーザー（Opus の推奨を承認） |
| 2026-11-20 | AG16K のビットストリームの埋め込みは、Supra の実サイズ（LZMA）が約 53 KB 以下で、かつ `list_root_dir` の削除と `opt-level="z"` を入れた場合に限って検討する | R81 の上限の表 | Opus |
| 2026-11-20 | `trim` を ASCII 専用の共用関数にする（全角スペースを削らなくなる差を受け入れる） | 2E のフラッシュ −0.5 KB。INI・GAME.CFG のキーは ASCII（R88） | Opus |
| 2026-11-20 | コメントだけの変更による defmt ありのビルドの 6 バイト差を受け入れる | defmt の番号の入れ替わりで動作は不変。`DEFMT_LOG=off` のビルドは同一（R88） | Opus |
| 2026-11-20 | R89 の確認済みの指摘をすべて直す（4並列、worktree） | ユーザーの指示「全部」（R90） | ユーザー |
| 2026-11-20 | CD_READ の SD 失敗は Error 0x34（MEDIUM ERROR）、既定のマウントの失敗は ILLEGAL REQUEST、副への 0x08・0xA0 は ABRT | 仕様に合わせる。実機で BIOS・メニューの反応を確認する（R90） | Fable 提案、Opus 承認 |
| 2026-11-20 | 取り決めに GPBR0 の「復旧中」の値・`LOCK_UNKNOWN`・書き直しの上限3回を追加し、`IFACE_GEN` は 1 のまま | 常駐部もロック済みの基板もまだ無い（R90、契約 §10） | Fable 提案、Opus 承認 |
| 2026-11-20 | `ack_command` の取りこぼし（A-c の残り）は FPGA の変更が要るので保留 | スロット 0 の書き込みで後から来たコマンドも消える（R90） | Opus |
| 2026-11-20 | Supra は実行せず静的解析だけ。動かすなら隔離した環境で `af.exe`・`yosys.exe` だけ、urjtag（`inpout32` のカーネルドライバ）は使わない | 署名が無く公式の配布物との一致も未確認（R92） | Opus |
| 2026-11-21 | VirusTotal への Supra のアップロードはユーザーが自分の手で行う | 拡張から shadow DOM のファイル欄に届かず、`yosys.exe` は 10 MB の上限も超える。API キーを Claude が扱うことはできない（R94） | Opus |
| 2026-11-21 | Supra の安全性は「静的には問題なし、Kaspersky も未検出」で一区切りにする。残りの確認（Tcl から実行時に呼ばれるもの、公式の配布物との一致）は、隔離環境で動かすときか、公式のハッシュが手に入ったときに行う | R95 | Opus |
| 2026-11-21 | NetGD 専用メニュー（`netgd-menu`）を作る。一覧はアプリが SD を読み、起動は `52 82` と BIOS への引き渡しで行う。ファームは変えない | 2E のフラッシュに余裕が無く、ファームに一覧を持たせない（R97） | ユーザー（実施）、Fable 設計 |
| 2026-11-21 | 2E の削減案 2 は、オプトインのプロファイル（`release-z`・`release-z-hot-s`）として入れる。既定にするかは実機で速度を比べてから | 既定のビルドを変えずに 7 KB 以上の余りを用意できる（R98） | Opus |
| 2026-11-21 | Supra は WSL の Linux 版を `unshare -rn`（ネットワークなし）で動かす。Windows 版とシステム設定を変える仮想化は使わない | 隔離ができて、ホストの設定を変えない（R99） | Opus |
| 2026-11-21 | AG16K のビットストリームは、大きさの面では埋め込みが成り立つ（LZMA 37.7 KB、しきい値 53 KB）。埋め込むか SD に置くかの最終判断は、PS の転送時間と実ピンでの再測定の後に行う。それまで R66 の判断（SD 優先）は変えない | R99。非圧縮で 4.42 Mbit あり、PS の時間が未知 | Opus（判断はユーザー） |
| 2026-11-21 | ack は番号つきにする（スロット 0 の bit 15 で新旧を見分け、互換を保つ）。slot 19 はビットごとのマスク、slot 5 は下位バイトだけ | 新旧のファームとビットストリームのどの組み合わせでも動く（R100） | Fable 設計、Opus 承認 |
| 2026-11-21 | v5.5 の `OpenGDEMU.rbf` を、CI の Quartus（上流の公開イメージ）で作った新しいものに差し替える | R100 の修正を実機で効かせるため。番号つき ack は新旧の組み合わせで動き、古い rbf は履歴から戻せる（R103） | Opus |
| 2026-11-21 | AG16K の手順は `fpga/agm/` に置く。Supra の出力（PLL のファイル）とビットストリームはコミットせず、仮のピンの出力には `-provisional` を付けてファームに拾わせない | 再配布を避け、基板に流せないものが紛れ込まないようにする（R106） | Fable 設計、Opus 承認 |
| 2026-11-21 | v5.5 の rbf は R105 のもの（番号つき ack と Status）。2E `resident,sd-fast` は `release-z` で作り、CI もそれで確かめる | resident は配布していない開発用の構成で、`release` では 788 B 入らない（R107・R111） | ユーザー |
| 2026-11-21 | 純正 GDEMU 6.0 の CDI の記録（R117）で決まった ATAPI 応答（REQ_STAT・GET_SCD・REQ_SES・CDI の TOC・REQ_MODE・REQ_ERROR）を純正に合わせる。CD_READ の種別検査・生セクタ・CHECK のレジスタ・GDEMU_VERSION は保留 | CDI については「実機と比べるまで変えない」の条件を満たした。保留分はデータ不足か影響が広い（R118） | Opus 提案、ユーザーが main への取り込みを指示 |
| 2026-11-22 | GDI の lead-out を最後のトラックの終わりにし、FAD 549300 への水増しをやめる。REQ_ERROR に ASC/ASCQ、REQ_STAT の繰り返し回数 0xF、IDENTIFY PACKET を純正の 80 バイトに | 純正 GDEMU 6.0 の記録（R119・R123・R124）。水増しの方が安全という前提には実測の裏付けが無かった（R125） | Opus |
| 2026-11-22 | 切替のあとの NOT READY（D13）と CHECK のレジスタ（D5）を純正に合わせる。中身の無いフォルダへの切替（D12）は純正に合わせて「ディスクなし」にし、存在しない番号は今のまま断る。純正が拒む GDI の形（D11）は受け付けたまま | D13・D5 は純正のデータがそろった。D12 はメニューが純正の動きを前提にしているため。存在しない番号の純正の動きは未確認（R123） | ユーザー（Opus の推奨を承認） |
| 2026-11-22 | 切替の窓は、GDEMU.INI がキーを設定していればその値、無ければ 150 + 150 ms | 純正の実測値（R119）。試験用 SD の INI が 150 / 150 だったので、純正は INI の値を使っている可能性が高い。キーが無いときの純正の値は未測定（R126） | Opus（実機での確認待ち） |
| 2026-11-22 | 存在しない番号（1〜999）への `52 82` は 01 をマウントする。01 が無ければ「ディスクなし」で選択は前のまま。999 を超える欠番は拒否のまま | 純正は reset_goto によらず 01（R128・R129）。01 が無いときと 999 超は純正未調査（R132） | Opus（ユーザーの「純正に合わせる」方針） |
| 2026-11-22 | マウント経路のスタック削減は、インライン展開をやめてフレームの重なりを無くす方法にする（static 化はしない） | 2 つのマウントが同時に起きないことを示す必要が無く、BSS も増えない。「呼び出し側で 1 回だけ呼ぶ」案は候補ごとのフォールバックと合わない（R131） | Opus |
| 2026-11-22 | スタックの最高水位の計測（`stack-hw`）は既定でオフの機能にする | 2E release-z のイメージが詰め物で +512 B になり、起動時の書き込みも足すため。実機で測れたら既定にするか決め直す（R133） | Opus |
| 2026-11-22 | CD_READ の期待セクタ種別を純正と同じく検査する（GDI に 0x28、CDI に 0x24 は 5/64/00） | GD-ROM と CDI の両方で純正の規則を確認した（R119・R134）。純正と本物のドライブも同じく断る | ユーザー（取り込みを指示） |
| 2026-11-22 | GDEMU.ini に時間のキーが無いときの切替の窓を 150 + 150 ms から 500 + 250 ms に改める（R126 の判断の訂正） | キーの無い SD で純正を測った値（R135） | Opus |
