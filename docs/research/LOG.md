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
