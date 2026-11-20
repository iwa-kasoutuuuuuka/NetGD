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
| 2026-11-20 | 上流から引き継いだ ATAPI 応答の疑い（R11 の D）は、実機と比べるまで挙動を変えない | 実機の純正 GDEMU が仕様や Flycast と違う応答をしている可能性がある（コード内コメント） | Opus |
| 2026-11-20 | GitHub の公開先は iwa-kasoutuuuuuka/NetGD（公開）、上流は `upstream` リモートに残す | ユーザーの指示 | ユーザー |
| 2026-11-20 | 番号フォルダ方式を採用し、設定が無ければフォルダ 01 をマウント。`0x52` で番号指定・次／前に対応 | 本家と GDmenu / openMenu との互換（R13・R14） | ユーザー（G3 を選択）、Opus 設計 |
| 2026-11-20 | `0x52 82` で番号が見つからないときは、今のディスクを外さずにエラーを返す | openMenu はエラーを見ずに READY を待つため | Opus |
| 2026-11-20 | プローブツールは別 PC の DreamSDK でビルドし、KOS 2.3.0（ユーザーのフォーク）に合わせる。純正には読み取り系コマンドだけを送る | このPCに KOS が無い。純正ファームを壊さないため（R19） | ユーザー（ビルド環境）、Opus（安全方針） |
| 2026-11-20 | F5（リセット後の再マウント）は RSTn の立ち下がりカウンタで判定し、SRST では動かさない | SRST は KOS の `g1ata` 初期化で出て両デバイスに効くため、再マウントすると書き込みが全部拒否される（R15・R16） | Opus |
