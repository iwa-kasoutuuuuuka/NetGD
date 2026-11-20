# G2 設計書：BBA 経由で SD カードにゲームイメージを書き込む

対象：`docs/PLAN.md` の G2-1。G2-2〜G2-5 の実装はこの設計書に従う。
状態：設計のみ。**この文書にある速度の数値は、出典を明記したもの以外すべて未計測の見積もり**で、実機検証（G2-5）で置き換える。

根拠の種類を次の記号で示す。

- **[コード]** このリポジトリのコードを読んで確認した（ファイル名と関数名を併記）。
- **[KOS]** KallistiOS の master ブランチのソースを読んで確認した（`kernel/arch/dreamcast/hardware/g1ata.c`、`addons/libkosfat/fs_fat.c`）。ユーザー環境の KallistiOS 2.3 で同じかは未確認。
- **[実機ノウハウ]** ユーザーの BBA 開発ツール（Dreamcast-Dev-Tools-for-BBA）の `KNOWHOW_ja.md` に「実機」または「資料」として書かれている内容。
- **[推定]** 上のどれでも確認できていない見積もり。

---

## 1. 結論（先に）

1. **ファイルシステムの持ち主は Dreamcast 側のアプリにする。** KallistiOS アプリ `netgd-recv` が、セカンダリ IDE ディスク（SD カード全体）の FAT32 を **FatFs（ChaN）+ KOS の `g1ata` ブロックデバイス**で直接書く。PC はファイルの中身を送るだけで、ブロックもクラスタも触らない。
2. **転送路は差し替え式にし、2 段階で作る。**
   - 第 1 段（T1）：**dcload-ip の `/pc/` から引き取る**。PC 側はユーザーの既存ツール（`dc_auto` / `dc_gui` / `dc-tool-ip`）をそのまま使い、新しく書くコードを最小にする。
   - 第 2 段（T2）：**単体で動く TCP プロトコル NGTP**。dcload なしで、SD 上のアプリを GDMenu などから起動して使える。
   - どちらも同じ「マニフェスト」と同じ受信コア（FS 書き込み・検証・確定・復旧）を使う。転送路ごとの差分は薄い層だけ。
3. **ファーム側の必須変更は小さい**（5 件。4 章の表の「必須」）。現状のままでは KOS から DMA が使われず、FLUSH CACHE が ABRT になり、ボタン操作で書き込み中の SD が壊れうる。

最大の不確定要素は **SD への書き込み速度（MCU 経由、未計測）** と **G1 バスのビット化け（上流で報告あり、未再検証）**。前者は律速点、後者はデータ破壊の原因になるので、設計の中で検出手段を用意する（4.5 節、8 章）。

---

## 2. 前提：コードで確認した現状

### 2.1 セカンダリ IDE ディスク

| 事実 | 根拠 |
|------|------|
| セカンダリは SD カード全体（MBR 込み）を見せる素の ATA ディスク。LBA n = カードのブロック n | [コード] `ide.rs` `handle_disk_read` / `handle_disk_write` |
| READ/WRITE SECTORS、READ/WRITE DMA、LBA48 版（0x24/0x25/0x34/0x35）に対応。CHS は拒否 | [コード] `ide.rs` `handle` |
| 書き込みは `media_present()` が真の間 ABRT。つまり **GD-ROM を排出しないと書けない** | [コード] `handle_disk_write` 冒頭、`MEDIA_PRESENT` |
| 排出は ATAPI `START STOP UNIT`（LoEj=1, Start=0）。`MEDIA_PRESENT=false`、CD_READ キャッシュ破棄、CDDA 停止 | [コード] `handle_packet` の `ATAPI_START_STOP_UNIT` |
| ロード（LoEj=1, Start=1）は `load_media` → `rescan_media` → `gdi::load_default`。カードを読み直してマウントし、UNIT ATTENTION を立てる | [コード] `load_media`、`rescan_media` |
| 名前指定マウントは `0xE0` IMAGE_CTL。`MOUNT_NAME`(5) で名前を 9 バイトずつ送り、`MOUNT`(2) で確定。プライマリ（GD-ROM）宛ての ATAPI パケット | [コード] `handle_image_ctl` |
| 書き込みは 1 コマンド = 1 回の CMD25（`hsmci::write_multi_begin` → `feed` → `end`）。セクタ単位で FIFO から取り出して送る | [コード] `handle_disk_write`、`embassy-atsam3/src/hsmci.rs` |
| `write_multi_begin` は `count > 0xFFFF` を拒否。一方 LBA48 の count=0 は 65536 と解釈される → **65536 セクタの書き込みは必ず失敗する** | [コード] `CommandSnapshot::count`、`write_multi_begin` |
| CMD25 の `feed` はポーリングの PIO（TXRDY を 1 語ずつ待つ）。DMAC は読み出し側（`read_blocks_dma`）にしか無い | [コード] `hsmci.rs` |
| SD のクロックは 24 MHz・4 bit。CMD6 のハイスピード切替はしていない | [コード] `sdcard.rs` `init` |
| IDENTIFY は LBA と LBA48 だけを公表。ワード 49 の DMA ビット、ワード 63（MWDMA）、ワード 88（UDMA）は 0 | [コード] `stage_identify_disk` |
| FLUSH CACHE（0xE7 / 0xEA）は未実装で「unhandled cmd」として ABRT | [コード] `handle` の `other` 分岐 |
| ERROR レジスタは ABRT ビットとセンスキー 4 bit しか保持しない（FPGA の制約） | [コード] `mcu/fwupdater/src/main.rs` の `Status` の注記 |
| FPGA のセカンダリ書き込み FIFO は 512 語（= 1 セクタ分 × 2）。PIO はセクタごとに DRQ を落として流量制御、DMA は FIFO の空きで DMARQ を出す | [コード] `fpga/amaranth/hw/ide_device.py` `SECONDARY_WRITE_FIFO_DEPTH`、`docs/AddressMapMCU.md` slot 19 |
| ファーム更新は同じセカンダリの `0xF9`。2 段階の鍵のあと RAM 上の `fwupdater` に移る | [コード] `fwupdate.rs`、`mcu/fwupdater` |

### 2.2 ファームが SD の内容について覚えているもの（キャッシュ）

ホストが同じ FAT32 を書き換えるので、ファームが持つ「カードの写し」を全部洗い出した。

| もの | 場所 | 中身 | 無効化・作り直しのタイミング |
|------|------|------|------------------------------|
| `CD_READ_BUF` / `CD_READ_BUF_LBA` / `CD_READ_BUF_LEN` | `ide.rs` | 直近に読んだイメージのセクタ（IP.BIN の先読みを含む） | `invalidate_cd_read_cache`：排出、ロード、ディスク読み出しの開始、ディスク書き込みの成功時 |
| `GDI`（`Option<Gdi>`） | `ide.rs` | マウント中イメージの記述子 | `rescan_media` / `SUB_MOUNT` / `service_swap_request` で置換。**排出では消えない**（`media_present()` が偽なので使われないだけ） |
| `TRACKS` | `gdi.rs` | トラック表（開始 LBA、ファイル名、サイズ） | `load_image` のたびに作り直し |
| `EXTENTS`（最大 96 本） | `gdi.rs` / `fat.rs` | 各トラックファイルの SD 上のブロック範囲。**FAT を通さず直接ブロックを読むための地図** | `map_track_extents`（`load_image` の中）で作り直し |
| `SELECTED_DIR` / `SELECTED_NAME` | `gdi.rs` | 実行時に選ばれたイメージ（8.3 名のパスと、ホストが使う名前） | `load_by_name` で更新、`select_default` で消去。排出では残る |
| `MOUNT_NAME` | `ide.rs` | `0xE0` で送られてきた名前の途中経過 | チャンク 0 で消去 |
| `DISK_SECTORS` | `ide.rs` | MBR から求めたカードの大きさ | 起動時に 1 回（`size_disk_from_mbr`） |

持っていないもの：FAT のジオメトリ（`fat::probe` は毎回読む）、`embedded_sdmmc::VolumeManager`（呼び出しごとに作る）、イメージの一覧（ファームは列挙しない）。

**結論**：ホストが書き終えたあと `load_media`（= LOAD、または `0xE0` MOUNT）を 1 回通せば、`CD_READ_BUF`・`GDI`・`TRACKS`・`EXTENTS` はすべて作り直される。危ないのは「排出中」の次の 2 点。

1. **ボタン（`button.rs`）**。`on_press` は `gdi::next_of_mounted` で `SELECTED_DIR`（古い 8.3 パス）をたどって SD を読む。これは `button_task` という別タスクで動く。`handle_disk_write` は CMD25 を開いたままセクタごとに `wait_host`（中で `yield_now`）を待つので、**その隙にボタンのタスクが CMD17 を出し、CMD25 の途中に割り込む**。さらに `service_swap_request` が `MEDIA_PRESENT=true` に戻すので、書き込みの途中で以後の書き込みが ABRT になる。
2. **排出したままのリセット**。`sweep` のリセット処理は CDDA 停止とディスク形式の再設定だけで、メディアを戻さない。転送中に本体をリセットすると、電源を切るまで「ディスクなし」のままになる。

### 2.3 コンソール側ソフトについて分かっていること

| 事実 | 根拠 |
|------|------|
| KOS の `g1ata` は G1 バスの**スレーブ**だけを扱う（マスターは GD-ROM）。NetGD のセカンダリがこれに当たる | [KOS] `g1ata.c` |
| DMA を使う条件は IDENTIFY ワード 63 のビット 2（MWDMA モード 2）。成り立てば SET FEATURES(0x03) で転送モードを設定する。ワード 88 は見ない | [KOS] |
| LBA48 はワード 83 ビット 10 で判定 | [KOS] |
| DMA のバッファは 32 バイト境界が必須 | [KOS] |
| PIO 書き込みはセクタごとに BSY が下がるのを待ってから 256 語送る。ファームのセクタごとの DRQ/BSY 制御と合う | [KOS]、[コード] |
| `g1_ata_flush` は FLUSH CACHE(0xE7) / FLUSH CACHE EXT(0xEA) を送る。ブロックデバイスの `flush` と `g1_ata_shutdown` から呼ばれる | [KOS] |
| BSY/DRQ 待ちにタイムアウトが無い。デバイスが固まるとアプリも固まる | [KOS] |
| `g1_ata_mutex_lock` / `unlock` があり、GD-ROM ドライバと同じセマフォを使う | [KOS]（ヘッダで公開されているかは 2.3 で要確認） |
| `libkosfat`（`fs_fat`）は書き込み可だが、**rename が無い、空き容量の問い合わせが無い、事前確保が無い**（書くたびに 1 クラスタずつ確保） | [KOS] `fs_fat.c` |
| BBA + dcload-ip 2.0.3 の転送は約 4.3〜5.5 MB/s、LAN アダプタ HIT-0300 は約 1 MB/s | [実機ノウハウ] 13 章（「資料」扱い。ELF 送信の速度で、`/pc/` 読み出しの速度ではない） |
| `/pc/` への大きな 1 回書き込み（128 KB）は dcload-ip が固まることがある。4 KB ずつ + `fflush` で安定。読み込みは大きくても問題は出ていない | [実機ノウハウ] 4 章 |
| dcload の呼び出し途中で止まると「noexec」になり、電源の入れ直しでしか直らない。ping が返っても dcload が空いているとは限らない | [実機ノウハウ] 3 章 |
| dcload-ip は認証も暗号化も無い。Windows 版 dc-tool-ip は `/pc/` でドライブ全体を見せる | [実機ノウハウ] 3 章 |
| `/cd/` リダイレクトは KallistiOS 2.3 で動かない | [実機ノウハウ] 5 章 |
| 本家 GDEMU の SD の慣習：フォルダは `01` からの連番（`01` は GDmenu）、MBR 付き FAT32、ルートに `GDEMU.ini` | [実機ノウハウ] 6 章 |
| NetGD のファームは連番ではなく**フォルダ名**でマウントし、既定値は `GDEMU.CFG` から読む。フォルダは 3 階層まで | [コード] `gdi.rs` `load_by_name`、`CONFIG_NAME`、`MAX_PATH_PARTS` |
| GDI のトラックファイル名は 8.3 形式でないとマウントできない（`ShortFileName::create_from_str`） | [コード] `gdi.rs` `sfn_from_bytes` |

---

## 3. 方式の比較と選定

### 3.1 速度の見積もり（どこが律速か）

| 区間 | 速度 | 根拠 |
|------|------|------|
| PC → DC（BBA、dcload-ip） | 4.3〜5.5 MB/s（ELF 送信） | [実機ノウハウ] |
| PC → DC（BBA、dcload-ip の `/pc/` 読み出し） | 未計測。ELF 送信と同じ仕組みでデータが届くので同程度と予想 | [推定] |
| PC → DC（BBA、KOS の TCP スタック） | 未計測。公開された実測値は見つからなかった。KOS に `examples/dreamcast/network/speedtest` があるので、それで測る | [推定] |
| PC → DC（LAN アダプタ HIT-0300） | 約 1 MB/s | [実機ノウハウ] |
| DC → MCU（G1、DMA） | 約 16 MB/s が上限 | [コード] `smc.rs` / `dmac.rs` の注記 |
| DC → MCU（G1、PIO） | 数 MB/s | [推定] |
| MCU → SD（CMD25、24 MHz 4 bit、ポーリング PIO） | **未計測**。バスの理論上限は 12 MB/s。読み出しは「約 2 MB/s」と注記がある（`hsmci.rs`）。CMD24 時代は約 340 KB/s が上限だった（`handle_disk_write` の注記）。現状は 0.5〜1.5 MB/s と予想 | [コード] の注記 + [推定] |

**律速は MCU → SD の書き込み**とみる。1 GB の GDI は、1 MB/s なら約 17 分、0.5 MB/s なら約 35 分。ネットワークはどの方式でも足りる見込みなので、**転送路は速さより壊れにくさと実装量で選ぶ**。ただしこの前提自体が見積もりなので、G2-2 の最初に計測する（9 章）。

メモリ：Dreamcast の RAM は 16 MB。受信バッファ 1 MB + FatFs + KOS で 3 MB に収まる。ファイル全体を RAM に載せる方式は取らない（CDI は 700 MB 級、GDI のトラックは 1 GB 級）。

### 3.2 「誰が FAT32 を書くか」の比較

| 案 | 内容 | 長所 | 短所 | 判定 |
|----|------|------|------|------|
| **A** | DC 側アプリが FS を持つ。PC はファイルを送る | ネットワークが切れても FS の更新は DC 内で完結する。PC 側が単純。ブロック書き込みが DC のローカル操作なので往復遅延が無い | DC 側に FAT の書き込みコードが要る（既製品を使う） | **採用** |
| **B** | DC はブロック中継だけ。PC が FS を持つ（ネットワークブロック方式） | DC 側が最小。FS のコードを PC でテストできる | FAT とディレクトリの更新が複数ブロックにまたがる間に回線が切れると FS が壊れる。Windows には標準の NBD クライアントが無く、PC 側に FAT ライブラリを組み込むことになる。メタデータ更新のたびに往復が発生する | 不採用 |
| **C** | ファーム（MCU）が FS を持つ。コンソールはベンダーコマンドでファイルを流し込む | FS の持ち主が 1 人になり、キャッシュの食い違いが原理的に起きない | MCU のスタックは約 2 KB（`gdi.rs` の注記）、`embedded-sdmmc` は 1 ブロックずつの書き込みで遅く、連続領域の確保もできない。フラッシュと RAM の余裕も小さい | 不採用（将来案） |

A の FS ライブラリは **FatFs（ChaN、R0.15 系）を採用**する。`libkosfat` を使わない理由は 2.3 節のとおりで、特に次の 3 つが決め手。

- `f_expand` で**連続領域を事前確保**できる。ファームの高速読み出しは `EXTENTS` 96 本に収まることが条件（`fat.rs` `MAX_EXTENTS`）なので、書く側で断片化させないことが読み出し性能に直結する。
- 事前確保すると、**データを書いている間は FAT もディレクトリも更新されない**。その間に電源が落ちても FS の構造は壊れない（8 章）。
- `f_getfree`、`f_rename`、`f_unlink`、`f_mkdir`、LFN がそろっている。

FatFs のライセンスは BSD 風の 1 条項で、GPL-3.0 の本リポジトリに同梱できる。

### 3.3 転送路の比較（コーディネータからの追加要件）

| 観点 | **T1：dcload-ip の `/pc/` から引き取る** | **T2：単体の TCP プロトコル（NGTP）** |
|------|------------------------------------------|----------------------------------------|
| PC 側に必要なもの | dc-tool-ip とユーザーの既存ツール。新規は「ジョブを作るスクリプト」と `dc_auto` のプリセット 1 つ | 専用 CLI（Python か Rust）。dcload は不要 |
| DC 側の実装量 | 小。`fopen("/pc/...")` と `fread` だけ。KOS のネットワークスタックは使わない | 中。KOS のソケット、探索、セッション、再接続 |
| 速度 | BBA で数 MB/s の見込み（未計測）。SD 書き込みより速ければ十分 | 未計測。KOS の TCP が SD 書き込みより遅い可能性がある |
| 固まる危険 | dc-tool-ip を途中で閉じると noexec → 電源の入れ直し。ただし本設計は電源断に耐える作り（8 章）で、続きから再開できる。`/pc/` への**書き込み**は 4 KB ずつに限定して回避する | KOS の `g1ata` にタイムアウトが無い点は共通。ネットワーク側は自前で再接続できる |
| 起動方法 | GDmenu → DCLOAD-IP → `dc-tool-ip -x netgd-recv.elf`。開発中の流れと同じ | SD 上の `netgd-recv` イメージをメニューから起動。PC には CLI だけ |
| 使い勝手 | `dc_gui` のユニットとして登録でき、複数台・ログ・通知がそのまま使える。一方、利用者に dcload-ip の環境が要る | 開発環境の無い利用者でも使える。OS を選ばない |
| 安全性 | 認証なし、PC のドライブ全体が見える。信頼できる LAN 専用 | 認証は画面に出す合言葉だけ。暗号化なし。信頼できる LAN 専用 |
| LAN アダプタ HIT-0300 | dcload-ip が対応済み。追加作業なし | KOS のドライバに依存。要確認 |

**選定：T1 を先に作り、T2 を後から足す。**

- G2 の判定基準（PC から送ったイメージが SD に保存され、そのまま起動できる）は T1 だけで満たせる。
- T1 は実機で確かめられた経路（dcload-ip 2.0.3 + KOS 2.3）の上に乗るので、未知の要素が「FS 書き込み」と「ファーム」だけになる。最初の実機検証で切り分けがしやすい。
- T2 は「本家 GDEMU にできないことを、誰でも使える形で」にするために必要。受信コアが T1 で固まってから足せば、追加は通信層だけで済む。
- 両方で同じマニフェスト（5.1 節）を使うので、PC 側の検査・正規化・CRC 計算のコードも共通になる。

T1 で守る作法（[実機ノウハウ] 3・4・7 章より）：

- `/pc/` からの読み出しは 64 KiB 単位（設定で変更可）。`/pc/` への書き込みは結果ファイルだけで、4 KiB ずつ + `fflush`。
- 進捗の `printf` は 1 秒に 1 回まで（1 回ごとに dcload の呼び出しになる）。
- 1 秒に 1 回 `reload.req` と `cancel.req` を見る。あれば、いまのチャンクを書き終えてチェックポイントを残し、`exit(0)` で dcload に戻る。**PC 側は dc-tool-ip を強制終了しない**（noexec の原因）。
- `KOS_INIT_FLAGS` に `INIT_NET` も `INIT_NO_DCLOAD` も入れない（BBA は dcload-ip が握ったままにする）。
- 例外時は KOS 形式のダンプを出して dcload に戻る。ウォッチドッグは「SD 書き込みが 60 秒進まない」で発動。

---

## 4. ファーム側の変更（G2-2）

### 4.1 変更一覧

| # | 区分 | 変更 | 場所 | 理由 |
|---|------|------|------|------|
| F1 | 必須 | **排出中はボタンを無効にする。** `on_press` は `media_present()` が偽なら何もしない（SD も読まない）。`service_swap_request` も偽なら要求を捨てる | `button.rs` `on_press`、`ide.rs` `service_swap_request` | 2.2 節の 1。書き込み中の CMD25 への割り込みと、勝手な再マウントを防ぐ |
| F2 | 必須 | **FLUSH CACHE（0xE7）と FLUSH CACHE EXT（0xEA）を成功で返す。** 各書き込みコマンドは CMD12 とビジー解除待ちまで終えてから完了を返すので、実処理は不要 | `ide.rs` `handle` | KOS の `g1_ata_flush` と FatFs の `CTRL_SYNC` が ABRT で失敗する |
| F3 | 必須 | **IDENTIFY で MWDMA モード 2 を公表する。** ワード 49 のビット 8（DMA 対応）、ワード 63 = `0x0407`。SET FEATURES(0x03) は今のまま受けるだけでよい | `ide.rs` `stage_identify_disk` | これが無いと KOS は DMA を使わない。DMA の書き込み経路自体は実装済み |
| F4 | 必須 | **65536 セクタの書き込みを通す。** `handle_disk_write` の中で 65535 セクタ以下の CMD25 に分ける | `ide.rs` `handle_disk_write` | 2.1 節。アプリは 256 セクタ以下しか出さないが、他のホスト（DreamShell など）で確実に失敗する |
| F5 | 必須 | **排出中にハードリセットを見たらメディアを戻す。** `sweep` のリセット処理で、`!media_present()` かつ書き込みセッション中でなければ `load_media` を呼ぶ | `ide.rs` `sweep` | 2.2 節の 2。転送中にリセットしても、電源を切らずにメニューへ戻れるようにする |
| F6 | 推奨 | **書き込みセッションのベンダーコマンド**（4.2 節）。`0xE0` IMAGE_CTL にサブコマンドを 3 つ足す | `ide.rs` `handle_image_ctl` | 排出・再マウント・状態取得を 1 つの手順にまとめ、保護領域（F7）と検査値（F8）の入れ物にする |
| F7 | 推奨 | **セッション中の保護領域。** 第 1 パーティションの開始 LBA より前（MBR とその後ろの隙間）への書き込みを ABRT。パーティション開始位置は `fat::probe` と同じ読み方で求める | `ide.rs` `handle_disk_write`、`fat.rs` | アプリや FatFs の不具合でパーティション表を壊さない。フォーマット機能は当面作らない |
| F8 | 推奨 | **書き込みデータの検査値。** `handle_disk_write` が FIFO から取り出したバイト列に対して FNV-1a（`ide.rs` に既存の `fnv1a`）を計算し、直前の書き込みコマンドの値を F6 の STATUS で返す | `ide.rs` `handle_disk_write` | PIO も MWDMA もバス上に CRC が無い。G1 のビット化けを**読み戻しなしで**検出する（4.5 節） |
| F9 | 性能 | **CMD25 の前に ACMD23（事前消去ブロック数の通知）を送る** | `hsmci.rs` `write_multi_begin` | 多くのカードで連続書き込みが速くなる。効果は計測で確認 |
| F10 | 性能 | **CMD25 の `feed` を DMAC 化する**（読み出し側の `read_blocks_dma` と同じ形） | `hsmci.rs`、`dmac.rs` | 1 語ずつの TXRDY ポーリングをやめる。FIFO → RAM → HSMCI の 2 段コピーのうち後段を MCU から外す |
| F11 | 性能 | **SD のハイスピードモード（CMD6、50 MHz）** | `sdcard.rs` `init` | バスの上限を 2 倍にする。読み出し（G3-4）にも効くので G3 と共同 |
| F12 | 性能 | **書き込みコマンド中の defmt ログを減らす**（コマンドごとに 4〜5 行出ている） | `ide.rs` `sweep`、`handle_disk_write` | FAT やディレクトリの更新は 1 セクタのコマンドが続くので、コマンドごとの固定費が効く |
| F13 | 修正 | `IMAGE_CTL` の VERSION を 3 に上げ、予約バイトに機能ビットを入れる（bit0 = 書き込みセッション、bit1 = 検査値、bit2 = DMA 公表） | `ide.rs` `OGDM_PROTOCOL_VERSION` | アプリが古いファームを見分けて、機能を落として動けるようにする |

F9〜F11 は**計測してから着手を決める**。目標は「SD への連続書き込み 1 MB/s 以上」（9 章）。届いていれば F10・F11 は G3-4 に回してよい。

FPGA の変更は**不要**の見込み。書き込み FIFO（512 語）と DMARQ の流量制御は現状で足りる。ただし slot 19 を `set_dma_mode` と `set_pio_flow_control` がそれぞれ全体書き込みする点（`PLAN.md` の地雷メモ）は、F10 で転送の並行度が変わるときに再確認する。

### 4.2 書き込みセッション（F6）

`0xE0` IMAGE_CTL（プライマリ宛て ATAPI パケット）にサブコマンドを足す。既存の 2・3・5 は変えない。

| `packet[1]` | 名前 | 引数 | 動作 |
|-------------|------|------|------|
| 6 | `WSESSION_BEGIN` | `packet[2..6]` = `"NGWS"`（誤発行よけ） | CDDA 停止、`MEDIA_PRESENT=false`、`invalidate_cd_read_cache`、`WRITE_SESSION=true`、統計のクリア。鍵が違えば ILLEGAL REQUEST |
| 7 | `WSESSION_END` | `packet[2]`：0 = マウントしない、1 = 直前の選択か既定値をマウント、2 = `MOUNT_NAME` で送ってある名前をマウント | `WRITE_SESSION=false` にしてから、指定に応じて `load_media` か `load_by_name`。UNIT ATTENTION を立てる。マウント失敗は ILLEGAL REQUEST（セッションは終わる） |
| 8 | `STATUS` | 割り当て長 `packet[3..=4]`（既存の `send_alloc`） | 下の 48 バイトを返す |

`STATUS` の応答（リトルエンディアン）：

| オフセット | 長さ | 内容 |
|------------|------|------|
| 0 | 4 | `"NGST"` |
| 4 | 1 | フラグ：bit0 メディアあり、bit1 書き込みセッション中、bit2 直前の書き込みコマンドが成功 |
| 5 | 3 | 予約（0） |
| 8 | 4 | セッション中に書いたセクタ数 |
| 12 | 4 | セッション中に失敗した書き込みコマンド数 |
| 16 | 4 | 直前の書き込みコマンドの開始 LBA |
| 20 | 4 | 直前の書き込みコマンドのセクタ数 |
| 24 | 4 | 直前の書き込みコマンドのデータの FNV-1a 32 bit |
| 28 | 4 | 第 1 パーティションの開始 LBA（保護領域の境界。0 = 不明） |
| 32 | 16 | 予約（0） |

セッションの決まり：

- セッション中もセッション外も、書き込みの条件は今と同じ「`media_present()` が偽」。**セッションを使わない既存のホスト（排出してから書く DreamShell など）はそのまま動く。**
- セッション中は F7 の保護領域が有効になる。セッション外の排出中は今までどおり全域に書ける。
- セッション中にハードリセットを見たら、セッションを終わらせて `load_media` を呼ぶ（F5 と同じ処理）。SRST（KOS の初期化で出うる）では終わらせない。ハードリセットと SRST は `HW_IN_RESET` と slot 16（RSTn のエッジ数）で見分ける。**本体のソフトリセット（A+B+X+Y+Start）で G1 の RSTn が動くかは未確認**（11 章の質問 4）。
- セッション中の `START STOP UNIT`（ロード）、`MOUNT`、ボタンは拒否または無視する。終わらせる手段は `WSESSION_END` とハードリセットだけ。
- セッション中の `0xF9`（ファーム更新）は拒否する。

現在マウント中の名前（`SELECTED_NAME`）はファームから返さない。名前は最大 `gamecfg::MAX_NEXT` 文字あって応答が大きくなるうえ、アプリは `GDEMU.CFG` を自分で読めるので、「上書きしてはいけないフォルダ」はアプリ側で判断する（5.5 節）。

### 4.3 「マウント中のイメージのフォルダに書かせない」の実現方法

ファームはブロックしか見えないので、フォルダ単位の禁止はファームではできない。次の 2 層で実現する。

1. **ファーム**：メディアがある間は書き込みを 1 セクタも通さない（既存の `handle_disk_write` 冒頭）。セッション開始 = 排出なので、「マウント中のイメージ」は存在しなくなる。
2. **アプリ**：`GDEMU.CFG` が指す既定のフォルダと、アプリ自身の起動元フォルダは、`--force` 相当の明示指定が無い限り上書きも削除もしない（5.5 節）。

### 4.4 書き込み後の再スキャン

`WSESSION_END`（1 か 2）→ `load_media` / `load_by_name` → `load_image` → `parse_image` + `map_track_extents`。これで 2.2 節の表のうち `GDI`・`TRACKS`・`EXTENTS`・`CD_READ_BUF`・`SELECTED_*` がすべて新しい内容になる。追加の無効化処理は要らない。確認すべきは次の 2 点で、どちらもホストテストにする（9 章）。

- `load_default` は `SELECTED_NAME` のフォルダが消えていたら `GDEMU.CFG` に落ちる（既存の実装どおり）。
- 同じ名前のフォルダを別のイメージで置き換えた場合、`load_by_name` が 8.3 パスを引き直す（`SELECTED_DIR` を使い回さない）。

`DISK_SECTORS` は起動時にしか求めない。アプリはパーティション表を書かない（F7 で禁止する）ので、作り直しは不要。

### 4.5 データの完全性：どの区間を何で守るか

| 区間 | 手段 |
|------|------|
| PC のファイル → DC の RAM | チャンクごとの CRC32（T2）。T1 は dcload-ip の UDP チェックサムに加えて、ファイル全体の CRC32 を DC 側で受信しながら計算してマニフェストの値と比べる |
| DC の RAM → G1 → MCU | F8 の FNV-1a。アプリは ATA 書き込みコマンドごとに、送ったデータの FNV-1a と `STATUS` の値を比べる。違えば同じ LBA に書き直す（3 回まで） |
| MCU → SD | SD のプロトコルが持つデータ線の CRC16（カードが検査し、`feed` / `end` がエラーを拾う） |
| 全体（任意） | 読み戻し検証。書いたファイルを `g1ata` で読み直して CRC32 を取る。初期の実機検証では既定で有効、安定したら既定で無効にする |

F8 が無いファーム（VERSION < 3）では、アプリは読み戻し検証を必須にする。

---

## 5. プロトコル

### 5.1 マニフェスト（T1・T2 共通）

PC 側が作り、DC 側が受け取る「これから送るイメージの目録」。UTF-8、LF 改行、1 行 1 項目。C で `sscanf` せずに先頭語と空白区切りで読める形にする。ファイル名は行の残り全部（空白を含んでよい）。

```
NETGD-MANIFEST 1
image Crazy Taxi (US)
type gdi
file 1234 89ABCDEF disc.gdi
file 1058400 01234567 track01.bin
file 1723344 0BADF00D track02.raw
file 1185760800 DEADBEEF track03.bin
descriptor disc.gdi
end
```

| 行 | 意味 |
|----|------|
| `NETGD-MANIFEST <版>` | 1 行目。版は整数。知らない版は拒否 |
| `image <SD 上のフォルダ>` | カードのルートからの相対パス、`/` 区切り、最大 3 階層（ファームの `MAX_PATH_PARTS`） |
| `type gdi` / `type cdi` | イメージの種類 |
| `file <バイト数> <CRC32 16 進 8 桁> <名前>` | フォルダ直下のファイル。サブフォルダは不可。最大 128 行 |
| `descriptor <名前>` | マウントの入口になるファイル（`.gdi` か `.cdi`）。`file` 行のどれかと同じ名前 |
| `end` | 終端。ここまで届かなければ不完全として拒否 |

CRC32 は zlib と同じもの（多項式 0xEDB88320、初期値と最終 XOR は 0xFFFFFFFF）。Python は `zlib.crc32`、Rust は `crc32fast`、C は 1 KiB の表で実装する。

PC 側が送る前に検査・正規化すること（DC 側でも同じ検査をして、通らなければ拒否する）：

- フォルダ名とファイル名は ASCII の印字可能文字だけ。`\ / : * ? " < > |` と先頭・末尾の空白、`.` `..` は不可。ファームの `MOUNT_NAME` は 1 バイト = 1 文字として扱う（`handle_image_ctl`）ので、**当面 ASCII 以外のフォルダ名はマウントできない**。
- フォルダ名は 127 バイト以下、パス全体は `gamecfg::MAX_NEXT` 以下。
- GDI のトラックファイル名は 8.3 形式にする（2.3 節）。長い名前は `track01.bin` の形に付け替え、`.gdi` の中身も書き換える。CRC はその後で計算する。
- 1 ファイルは 4 GiB 未満（FAT32 の上限）。
- `.gdi` に書かれたトラックファイルがすべて `file` 行にあること。

### 5.2 SD 上の置き方と「原子的な確定」

イメージのフォルダを `<dir>` とする。

| 段階 | SD の状態 |
|------|-----------|
| 1. 開始 | `<dir>/` を作る。`<dir>/NETGD.MF`（マニフェストの写し）と `<dir>/NETGD.CKP`（512 バイト固定のチェックポイント）を書いて `f_sync` |
| 2. 確保 | 全ファイルを作り、`f_expand(fp, size, 1)` で連続領域を確保して `f_sync`。**記述子だけは拡張子を `.NGP` にして作る**（`disc.gdi` → `DISC.NGP`）。記述子の基本名は 8 文字以内の ASCII に付け替える |
| 3. 転送 | 各ファイルにデータを書く。FAT とディレクトリは更新されない。8 MiB ごとに `NETGD.CKP` を上書き（1 セクタの書き込み 1 回） |
| 4. 検証 | 全ファイルの CRC32 がマニフェストと一致することを確認（受信しながらの計算。指定があれば読み戻しも） |
| 5. 確定 | `NETGD.CKP` に「完了」を書く → 記述子のディレクトリエントリの拡張子 3 バイトを `NGP` から `GDI`/`CDI` に書き換える（**1 セクタの書き込み 1 回**）→ `NETGD.CKP` を削除 → `f_sync` |

なぜこれで安全か：

- ファームは拡張子が `GDI` か `CDI` のファイルがあるフォルダだけをマウントする（`gdi.rs` `find_gdi`、`cdi.rs` `find_cdi`）。**`.NGP` の間は、どのメニューからもファームからも「イメージ」に見えない。**
- 確定は 32 バイトのエントリの中の 3 バイトの書き換えで、SD の 1 ブロックに収まる。書き込みの途中で電源が落ちても、結果は「前」か「後」のどちらか。
- FatFs の `f_rename` は「新しいエントリを作ってから古いエントリを消す」ので 2 回の書き込みになり、間で落ちると同じクラスタ列を指すエントリが 2 つ残る。そのため `f_rename` は使わず、`FIL` 構造体の `dir_sect` と `dir_ptr` からエントリの位置を得て、そのセクタを読み・3 バイト直し・書く小さな関数（`ngfs_commit_ext`）を自前で持つ。記述子を 8.3 名に限るのは、LFN エントリのチェックサム（基本名と拡張子から計算する）を直さずに済ませるため。
- CDI は記述子 = イメージ本体なので、700 MB の本体が `.NGP` の名前で書かれ、最後に拡張子だけが変わる。

`NETGD.CKP`（512 バイト、リトルエンディアン）：

| オフセット | 長さ | 内容 |
|------------|------|------|
| 0 | 4 | `"NGCK"` |
| 4 | 2 | 版（1） |
| 6 | 2 | 状態：1 = 確保中、2 = 転送中、3 = 完了（確定の直前） |
| 8 | 4 | マニフェスト全体の CRC32（別のイメージの残骸と取り違えないため） |
| 12 | 2 | 転送済みのファイル数（先頭から順に全部書き終えた数） |
| 14 | 2 | 予約 |
| 16 | 8 | いま書いているファイルの、SD に書き終えたバイト数（8 MiB の倍数） |
| 24 | 4 | そこまでの CRC32 の途中状態 |
| 28 | 480 | 予約（0） |
| 508 | 4 | 0〜507 の CRC32 |

再開：DC 側は `NETGD.MF` と `NETGD.CKP` を読み、送られてきたマニフェストの CRC32 が一致すれば、チェックポイントの位置から続ける。一致しなければ「別物が途中まである」として、上書き指定が無い限り拒否する。`NETGD.CKP` 自体が壊れていたら（末尾の CRC 不一致）、そのフォルダは先頭からやり直す。

`NETGD.MF` は確定後も残す（後からの検証に使う。数百バイト）。残したくない場合の扱いは 11 章の質問 6。

### 5.3 T1：dcload-ip の `/pc/` 引き取り

ネットワーク上の取り決めは dcload-ip のものをそのまま使い、その上に**ファイルの置き方**だけを決める。DC 側が主導して読みに行く。

PC 側のジョブフォルダ（`dc-tool-ip` を起動したドライブの上。パスに空白と日本語を入れない）：

```
<ドライブ>:\netgd\<ユニット名>\
    job.ngj         実行する操作の並び（下記）
    0001\manifest.ngm   1 つ目のイメージのマニフェスト
    0001\src.txt        マニフェストの file 行と同じ順で、PC 上の元ファイルの /pc/ パスを 1 行ずつ
    0002\...
    cancel.req      あれば中断（PC 側が作る）
    result.ngr      結果（DC 側が書く。4 KiB ずつ + fflush）
```

`job.ngj`（UTF-8、1 行 1 操作、上から順に実行）：

```
NETGD-JOB 1
statfs
list /
put 0001 resume
put 0002 overwrite
delete Old Game (JP)
verify Crazy Taxi (US)
finish mount Crazy Taxi (US)
```

| 操作 | 意味 |
|------|------|
| `statfs` | 空き容量と総容量を結果に書く |
| `list <フォルダ>` | フォルダの中身と、各イメージの状態（完了 / 途中 / イメージでない）を結果に書く |
| `put <番号> [resume\|overwrite\|new]` | `<番号>\manifest.ngm` のイメージを書く。`new`（既定）は同名フォルダがあれば失敗、`resume` は途中から、`overwrite` は消してから |
| `delete <フォルダ>` | イメージのフォルダを消す（直下のファイルだけ。サブフォルダがあれば失敗） |
| `verify <フォルダ>` | `NETGD.MF` と照らして読み戻し検証 |
| `finish [mount <フォルダ>\|default\|none]` | セッションを終える。指定に応じて再マウント（既定は `default`） |

コンソールへの出力は、PC のスクリプトが拾えるように行頭を固定する（1 秒に 1 回まで）：

```
NGD:HELLO app=1.0.0 fw=OGDM/3 caps=7 free=123456789012 total=499999999999
NGD:PUT 0001 file=3/4 bytes=104857600/1185760800 rate=912000
NGD:DONE 0001 ok
NGD:ERROR 0002 code=ENOSPC need=734003200 free=10485760
NGD:FINISH ok mounted="Crazy Taxi (US)"
```

`result.ngr` は同じ内容の全量（`list` の結果を含む）。終了コードは、全操作成功で 0、1 つでも失敗で 1。

### 5.4 T2：NGTP（単体 TCP）

**探索**（UDP、ポート 20039）：PC が `NGTP-DISCOVER 1\n` をブロードキャストする。DC は送信元に `NGTP-HERE 1 name=<名前> app=<版> state=<idle|busy>\n` を返す。直結で IP が分かっていれば探索は省ける。

**接続**：DC がサーバ（TCP、ポート 20039）。同時接続は 1 本。2 本目は `BUSY` を返して切る。

**フレーム**（すべてリトルエンディアン）：

| オフセット | 長さ | 内容 |
|------------|------|------|
| 0 | 4 | `"NGT1"` |
| 4 | 1 | 種別（下表） |
| 5 | 1 | フラグ（予約、0） |
| 6 | 2 | 予約（0） |
| 8 | 4 | 通し番号（送信側ごとに 1 ずつ増える。応答は要求の番号を返す） |
| 12 | 4 | ペイロード長 n（最大 65536 + 16） |
| 16 | n | ペイロード |
| 16+n | 4 | 先頭からここまでの CRC32 |

CRC が合わないフレームを受けたら、`ERROR EBADCRC` を返して接続を切る（TCP の上なので、ここで合わないのは RAM かドライバの異常。再送で取り繕わない）。

**メッセージ**：

| 種別 | 名前 | 向き | ペイロード | 応答 |
|------|------|------|-----------|------|
| 0x01 | `HELLO` | PC→DC | プロトコル版 u16、合言葉 u16（画面に出ている 4 桁）、クライアント名 | `HELLO_OK`：プロトコル版、アプリ版、ファームの VERSION 応答 32 バイト、機能ビット、最大チャンク長、セクタ/クラスタの大きさ |
| 0x02 | `STATFS` | PC→DC | なし | `STATFS_OK`：総バイト数 u64、空きバイト数 u64、空きが概算かどうか u8 |
| 0x03 | `LIST` | PC→DC | フォルダのパス | `LIST_OK`：項目の並び（種類、状態、サイズ u64、名前）。64 KiB を超えるなら「続きあり」を立てて複数フレーム |
| 0x04 | `BEGIN` | PC→DC | モード u8（new / resume / overwrite）、マニフェスト全文 | `BEGIN_OK`：ファイルごとの再開位置 u64 の並び。この時点で確保（5.2 節の 2）まで終わっている |
| 0x05 | `DATA` | PC→DC | ファイル番号 u16、予約 u16、オフセット u64、データ（最大 64 KiB） | 個別には返さない。DC は SD に書き終えた位置を `ACK`（ファイル番号、オフセット）で 1 MiB ごとに知らせる |
| 0x06 | `FILE_END` | PC→DC | ファイル番号 u16 | `FILE_OK`（CRC32 一致）または `ERROR ECRC` |
| 0x07 | `COMMIT` | PC→DC | 読み戻し検証の有無 u8 | `COMMIT_OK`。5.2 節の 4・5 を実行 |
| 0x08 | `ABORT` | PC→DC | 残すか消すか u8 | `ABORT_OK`。残した場合は後で `resume` できる |
| 0x09 | `DELETE` | PC→DC | フォルダのパス | `DELETE_OK` |
| 0x0A | `VERIFY` | PC→DC | フォルダのパス | `VERIFY_OK`：ファイルごとの結果 |
| 0x0B | `FINISH` | PC→DC | 再マウントの指定 u8、名前 | `FINISH_OK`。4.2 節の `WSESSION_END` を実行 |
| 0x0C | `PING` | 双方向 | なし | `PONG` |
| 0x7F | `ERROR` | DC→PC | コード u16、メッセージ（ASCII） | — |

**流量制御**：`DATA` は TCP の受信ウィンドウに任せる。DC は受信リング（1 MiB）が埋まれば `recv` を止め、それで PC の送信が止まる。PC は `ACK` を待たずに送り続けてよいが、`FILE_END` の前に最後の `ACK` を確認する。

**再開**：接続が切れたら、PC は再接続して同じマニフェストで `BEGIN resume` を送る。`BEGIN_OK` の再開位置（チェックポイントの位置、8 MiB 単位）から `DATA` を送り直す。PC は再開位置までの CRC を自分で計算し直す必要は無い（途中状態は DC が `NETGD.CKP` に持っている）。

**版**：`HELLO` の版が DC の対応範囲に無ければ `ERROR EVERSION`（対応範囲をメッセージに入れる）。フレームの種別やペイロードの意味を変えるときは版を上げる。ペイロードの末尾に欄を足すだけなら版は上げず、受信側は知らない末尾を無視する。

**エラーコード**（T1 の `NGD:ERROR code=` と共通）：`EVERSION`、`EAUTH`、`EBUSY`、`EBADCRC`、`EMANIFEST`（検査に通らない）、`EEXIST`、`ENOENT`、`ENOSPC`、`EFRAG`（連続領域が取れず、断片化を許す指定も無い）、`ECRC`（ファイルの CRC 不一致）、`EIO`（ATA か SD のエラー）、`EBUS`（F8 の検査値が 3 回合わない）、`EPROTECTED`（5.5 節）、`EFW`（ファームが古い、またはセッションを開けない）、`ESTATE`（順序違い）。

### 5.5 アプリが守る安全の決まり

- 書くのはカードの**第 1 パーティションの FAT32 だけ**。FAT32 でなければ（exFAT、FAT16）何もせずにエラー。フォーマット機能は持たない。
- `GDEMU.CFG` が指す既定のフォルダ、アプリ自身の起動元フォルダ、`01`（本家の慣習で GDmenu）は、明示の強制指定が無い限り上書き・削除しない（`EPROTECTED`）。
- `delete` は、`NETGD.MF` があるか、`.gdi`/`.cdi`/`.NGP` があるフォルダだけを対象にする。ルートは消せない。
- 空きが「必要量 + 16 MiB」に満たなければ `BEGIN` の時点で `ENOSPC`。確保（`f_expand`）が先に全部終わるので、**転送の途中で満杯になることは無い**。
- 連続領域が取れないときは既定で `EFRAG`。PC 側の `--allow-fragmented` で、連続でない確保に落として続行できる（そのイメージは読み出しが遅い経路になりうる、と警告を出す）。

---

## 6. コンソール側アプリ `netgd-recv`（G2-3）

### 6.1 置き場所と構成

```
dc/netgd-recv/
    Makefile            KOS 用（kos-cc）。ユーザー環境の KallistiOS 2.3 / DreamSDK を基準にする
    Makefile.host       受信コアを PC の gcc でビルドするテスト用
    src/
        main.c          起動、画面、状態遷移
        atapi.c         プライマリ宛ての生 ATAPI パケット（g1_ata_mutex_lock の中で発行）
        fwctl.c         0xE0 IMAGE_CTL：VERSION / WSESSION_BEGIN / END / STATUS / MOUNT_NAME / MOUNT
        diskio_g1.c     FatFs の diskio：g1_ata_read_lba / write_lba(_dma) / flush、F8 の照合と書き直し
        diskio_file.c   同じ diskio をイメージファイルで実装（ホストテスト用）
        ngfs.c          FS 操作：確保、チェックポイント、確定（ngfs_commit_ext）、復旧、削除、一覧
        manifest.c      マニフェストの解析と検査
        recv_core.c     受信コア：put / resume / verify / finish の状態機械（転送路に依存しない）
        xport_pc.c      T1：/pc/ からの引き取り、job.ngj の解釈、NGD: 行の出力
        xport_ngtp.c    T2：UDP 探索、TCP サーバ、フレーム
        crc32.c fnv1a.c
    third_party/fatfs/  FatFs R0.15 系（ff.c、ffconf.h、ffunicode.c）
    romdisk/            フォント程度。起動後に GD-ROM を読まないため、必要なものは全部ここ
```

FatFs の設定：`FF_USE_EXPAND=1`、`FF_USE_LFN=2`、`FF_FS_EXFAT=0`、`FF_FS_NOFSINFO=0`（FSInfo の空き数を信用する。500 GB のカードで FAT 全体を数えると数十秒かかるため。空きは「概算」として表示し、足りないと判定したときだけ数え直す）、`FF_FS_READONLY=0`、`FF_MAX_SS=512`。

### 6.2 処理の流れ

1. 起動。`dcload_type` で起動元を判定（`DCLOAD_TYPE_IP` なら T1、それ以外は T2）。
2. `0xE0` VERSION でファームを確認。`OGDM` が返らなければ「NetGD のファームではない」と表示して終了（本家 GDEMU では何もしない）。
3. `g1_ata_init` → セカンダリを検出。見つからなければ終了。
4. `WSESSION_BEGIN`（古いファームなら `START STOP UNIT` の排出で代用）。
5. FatFs でマウント。全フォルダは走査せず、ルートと指定フォルダだけを見る。途中のイメージ（`NETGD.CKP` がある）は一覧に「途中」と出す。
6. 転送路の指示に従って `put` / `delete` / `verify` を実行。
7. `f_sync` → `f_unmount` → `g1_ata_flush` → `WSESSION_END`。
8. T1 は `exit(0)` で dcload に戻る。T2 は「A でメニューに戻る」を表示し、`arch_menu()` で BIOS に戻る（BIOS は再マウントされたディスクを起動する）。

ATA 書き込みは 1 コマンド 256 セクタ（128 KiB）以下、32 バイト境界のバッファ、DMA が使えれば `g1_ata_write_lba_dma`。

スレッド：T1 は 1 本（読む → 書くの繰り返し。dcload の呼び出し中は割り込みが止まるので、G1 の DMA と重ねない）。T2 は受信スレッドと書き込みスレッドの 2 本を 1 MiB のリングでつなぐ。

### 6.3 起動方法

| 方法 | 用途 | 手順 |
|------|------|------|
| dcload-ip | 開発、T1 | GDmenu → DCLOAD-IP → PC から `dc-tool-ip -x netgd-recv.elf`（ユーザーの `dc_run.ps1` / `dc_auto.ps1` 経由） |
| SD 上のイメージ | 配布、T2 | `NETGD` フォルダに CDI（MIL-CD 形式）を置き、メニューから選ぶ。起動後はディスクを読まないので、排出しても動き続ける |
| 自己更新 | 配布 | `netgd-recv` 自身の新しい CDI も普通のイメージとして `put` できる（起動元フォルダの保護は強制指定で外す） |

`0xE0` を使うメニュー（NetGD 用の openMenu / GDMenu 相当）からの起動は G3-1 の差分表で扱う。

### 6.4 画面

文字だけ（KOS の `bfont` か `conio`）。1 画面に、ファームの版、IP アドレスと合言葉（T2）、カードの空き、いまの操作、ファイル名、進捗バー、速度、残り時間、エラーを出す。操作は「B で中断（チェックポイントを残して安全に止める）」と「A で終了」だけ。

---

## 7. PC 側ツール（G2-4）

### 7.1 置き場所と構成

```
tools/netgd-send/
    netgd_send/             Python 3（標準ライブラリのみ）
        __main__.py         CLI
        image.py            GDI / CDI の検査、トラック名の 8.3 正規化、.gdi の書き換え
        manifest.py         マニフェストの生成と解析、CRC32
        job.py              T1：ジョブフォルダの作成、result.ngr の解釈
        ngtp.py             T2：探索、フレーム、put / resume
    tests/                  9 章のハードウェアなしテスト
    integrations/dcdevtools/
        netgd.ps1           Dreamcast-Dev-Tools-for-BBA 用のプリセット
        README.md           プリセットの入れ方
```

Python にする理由：T1 は PowerShell から呼ぶだけ、T2 は `socket` と `zlib` で足りる。配布形態（単体 exe）が要るようになったら Rust に書き直す。プロトコルは 5 章で固定してあるので、言語は後から替えられる。

CLI：

```
netgd-send put <イメージのフォルダか .gdi/.cdi> [--dest "<SD 上のフォルダ>"] [--resume|--overwrite]
               [--verify-readback] [--allow-fragmented] [--mount]
netgd-send list [<フォルダ>]
netgd-send delete "<SD 上のフォルダ>"
netgd-send verify "<SD 上のフォルダ>"
netgd-send statfs
共通：--transport pc|ngtp   --ip <DC の IP>   --unit <名前>   --jobroot <ドライブ:\netgd>   --code <合言葉>
```

`--transport pc`（T1）のとき、`netgd-send` 自身は dc-tool-ip を起動しない。`--prepare` でジョブフォルダを作って終了し、`--collect` で `result.ngr` を読んで結果を表示する。**送信・実行・ログ・noexec の検出は、すべてユーザーの既存ツールに任せる**（重複させない）。

### 7.2 Dreamcast-Dev-Tools-for-BBA への組み込み

ユーザーのリポジトリには、`profiles\*.ps1` のプリセット（ELF、ビルドコマンド、監視フォルダ、`ReloadReq`、`PostRun`、`Exclusive`）と、`config\units.json` のユニット（名前、IP、プリセット）がある（README より。プリセットの全項目は未確認）。組み込みは**プリセット 1 つの追加だけ**にする。

| プリセットの項目 | 設定 |
|------------------|------|
| ELF | NetGD リポジトリの `dc/netgd-recv/netgd-recv.elf` |
| ビルドコマンド | `python -m netgd_send put ... --transport pc --prepare --unit <ユニット名>`（ジョブフォルダを作る。ELF のビルドは別途） |
| `PostRun` | `python -m netgd_send --collect --unit <ユニット名>`（`result.ngr` を読んで成否を表示・通知） |
| `ReloadReq` | 有効。`netgd-recv` は `reload.req` を見て、チェックポイントを残して安全に `exit` する |
| `Exclusive` | ユニット名（ジョブフォルダをユニットごとに分けるので、台をまたぐ排他は不要） |
| 監視（`-Watch`） | 使わない（転送は 1 回きり） |

- ジョブフォルダは `<ドライブ>:\netgd\<ユニット名>\` とし、台ごとに分ける（[実機ノウハウ] 10 章）。元のイメージファイルはコピーせず、`src.txt` に `/pc/` パスで書く。**元ファイルが dc-tool-ip の起動ドライブと別のドライブにあると読めない**ので、`dc_run.ps1` の `-PcRoot` で合わせるか、同じドライブに置く。パスに空白や日本語があるときは、ジョブフォルダの中に ASCII 名のハードリンクを作る（同一ドライブなので可能）。
- 進捗は `NGD:` 行がそのまま `dc_auto` のログと `dc_gui` に流れる。
- このプリセットは NetGD 側（`tools/netgd-send/integrations/dcdevtools/netgd.ps1`）に置き、ユーザーのリポジトリには「`profiles\` にコピーするか、パスを指定して読む」だけにする。プリセットから外部のフォルダの ELF を指せるか、ビルドコマンドに引数を渡せるかは未確認（11 章の質問 3）。

---

## 8. 失敗の種類と復旧

| 失敗 | 何が起きるか | 復旧 |
|------|--------------|------|
| **データ転送中の電源断** | FAT とディレクトリは確保の時点で書き終わっているので、構造は壊れない。途中のファイルは「正しい大きさだが後ろが未書き込み」。記述子が `.NGP` なのでマウントされない | 次回、`put --resume` でチェックポイント（最大 8 MiB 手前）から続き。または `delete` |
| **確保中（`f_expand`）の電源断** | FAT の一部だけが書かれ、クラスタの取りこぼし（どのファイルにも属さない確保済みクラスタ）が残りうる。既存のファイルは壊れない | フォルダを `delete` → やり直し。取りこぼしは空きが減るだけ。PC の `chkdsk` で回収できる。アプリに回収機能を持たせるかは後で判断 |
| **確定の途中の電源断** | 拡張子の書き換え（1 セクタ）の前なら「途中」、後なら「完了」。`NETGD.CKP` が残っていて状態が「完了」なら、起動時にアプリが拡張子を確認して片付ける | 自動（アプリの起動時の復旧処理） |
| **削除の途中の電源断** | ファイルが一部消えたフォルダが残る。クラスタの取りこぼしがありうる | もう一度 `delete` |
| **カードが満杯** | `BEGIN` の時点で `ENOSPC`。SD には何も書かれない（フォルダも作らない） | 何か消してやり直し |
| **断片化で連続領域が取れない** | `EFRAG` | `--allow-fragmented`、または PC でカードを整理 |
| **ネットワーク切断（T2）** | DC は 30 秒待って、リングの残りを書き、チェックポイントを残して待ち受けに戻る | PC が再接続して `BEGIN resume` |
| **dc-tool-ip の異常終了・PC のスリープ（T1）** | `/pc/` の読み出しが返らず、アプリが dcload の呼び出しの中で止まる（noexec 状態） | 本体の電源を入れ直す → 「データ転送中の電源断」と同じ扱いで再開。これが T1 の弱点で、T2 を作る理由の 1 つ |
| **SD の書き込みエラー** | ファームが ABRT を返す。KOS の `g1ata` がエラーを返し、FatFs が `FR_DISK_ERR` | 同じコマンドを 3 回まで再試行。だめなら `EIO` で中断（チェックポイントは最後に成功した位置） |
| **G1 のビット化け** | F8 の検査値が合わない | 同じ LBA に書き直し（3 回まで）。だめなら `EBUS`。**メタデータ（FAT・ディレクトリ）の書き込みにも同じ照合をかける**。ここが化けると FS が壊れるため |
| **ファームが応答しない** | KOS の `g1ata` にタイムアウトが無いので、アプリが固まる | `diskio_g1.c` では KOS の関数を呼ぶ前後でウォッチドッグを動かし、60 秒で `arch_abort()`。根本対策として、自前のタイムアウト付き PIO/DMA 書き込みを持つかは実機で判断 |
| **転送中に本体をリセット** | F5 によりファームがメディアを戻す。SD は「データ転送中の電源断」と同じ状態 | 再開 |
| **転送中にボタンを押す** | F1 により無視される | — |
| **CRC 不一致（ファイル全体）** | `ECRC`。確定しない | そのファイルだけ先頭から送り直す（`resume` はチェックポイントを捨てる） |
| **FAT が元から壊れている** | FatFs のマウントか確保で失敗 | アプリは直さない。PC で `chkdsk` |

FAT の 2 本目（バックアップ FAT）は FatFs が 1 本目と同時に更新する。FSInfo の空き数は `f_sync` / `f_unmount` で書かれるので、電源断のあとは実際とずれることがある（空きが多めにも少なめにも出うる）。アプリは「前回のセッションが正常終了していない」（`NETGD.CKP` が残っている）ときは空きを数え直す。

---

## 9. マイルストーンと受け入れ試験

### 9.1 ハードウェアなしでできる試験

| 対象 | 方法 |
|------|------|
| マニフェスト、GDI/CDI の検査、8.3 正規化、CRC | `tools/netgd-send/tests`（Python の単体テスト） |
| NGTP のフレームと状態遷移 | PC 上で `netgd-send` と、ホスト向けにビルドした受信コア（`Makefile.host`、TCP はループバック）を対向させる |
| FS 操作（確保、チェックポイント、確定、削除、復旧） | 受信コア + `diskio_file.c` で、ファイル上の FAT32 イメージ（MBR 付き、`mkfs.fat` か FatFs の `f_mkfs` で作る）に書く。終了後に `fsck.fat -n` が無傷を報告すること、PC でマウントして全ファイルの CRC が合うこと |
| 電源断 | `diskio_file.c` に「N 回目のセクタ書き込み以降を全部捨てる」機能を入れ、N を 1 から最後まで総当たりする。どの N でも (a) `fsck.fat` が構造の破損を報告しない（取りこぼしクラスタは許す）、(b) 記述子が `.gdi`/`.cdi` なら全ファイルの CRC が合う、(c) 復旧処理のあと `resume` で完走する |
| ファームから見た結果 | 書き上がったイメージファイルを、P4 で切り出す `common/discimg`（`gdi::parse`、`cdi::parse`）と `fat.rs` 相当のエクステント計算に通す。マウントできること、エクステントがイメージ全体で 96 本以下であること |
| ファームのインターロック（F1・F5・F6・F7） | セッションの状態遷移を引数だけで動く関数に切り出し、ホストテストにする（P4 と同じやり方）。IDENTIFY のワード（F3）と count の分割（F4）も同様 |
| FPGA | 既存の `fpga/amaranth/test.py` に、セカンダリへの 256 セクタの WRITE DMA（MCU 側の取り出しを遅らせて DMARQ が下がる場合を含む）と、PIO 書き込みの連続コマンドを足す |

エミュレータ（Flycast など）は G1 のスレーブ ATA ディスクを持たないので、アプリ全体の試験には使えない。

### 9.2 マイルストーン

| 段階 | 内容 | 受け入れ試験 |
|------|------|--------------|
| **G2-2a** ファーム必須分 | F1〜F5、F13 | ホストテスト全通過。FPGA シミュ全通過。ファームのビルド成功。**実機**：KOS の `g1_ata_init` がセカンダリを DMA 可として認識する、`g1_ata_flush` が成功する |
| **G2-2b** 計測 | アプリの自己診断（一時ファイルを連続確保して 64 MiB 書き、読み戻す）で、PIO と DMA の書き込み速度を測る | 数値を本書 3.1 節に書き戻す。1 MB/s 未満なら F9 → F10 → F11 の順に着手し、そのつど測る |
| **G2-2c** ファーム推奨分 | F6〜F8、F12 | ホストテスト。**実機**：セッション中のボタン・リセット・MBR への書き込みが設計どおり扱われる。わざと 1 bit 変えたデータで F8 の不一致が出る（テスト用のビルドで） |
| **G2-3a** 受信コア（ホスト） | `manifest.c`、`ngfs.c`、`recv_core.c`、`diskio_file.c` | 9.1 節の FS・電源断・ファームから見た結果の試験を全通過 |
| **G2-3b** T1 実機 | `atapi.c`、`fwctl.c`、`diskio_g1.c`、`xport_pc.c` | **実機**：`/pc/` の読み出し速度を測る。100 MiB の試験ファイルを書き、読み戻しの CRC が合う |
| **G2-4a** PC ツール（T1） | `image.py`、`manifest.py`、`job.py`、プリセット | 単体テスト全通過。`dc_auto` から 1 コマンドで送信 → 結果表示まで通る |
| **G2-5a** T1 の通し検証 | GDI 1 本（1 GB 級）と CDI 1 本 | **実機**：送信 → 確定 → 再マウント → BIOS から起動してプレイできる。SD を PC に挿して `chkdsk` が無傷を報告し、全ファイルの CRC が合う。転送速度を記録。転送の途中で電源を切る → 再開 → 起動、を 3 か所（確保中、データ中、確定の直前）で行う |
| **G2-3c / G2-4b** T2 | `xport_ngtp.c`、`ngtp.py`、画面、CDI 化 | ホストの対向試験全通過。**実機**：KOS の TCP の速度を測る。dcload なしで、メニューから起動 → 送信 → 起動 |
| **G2-5b** T2 の通し検証 | G2-5a と同じ内容に加えて、LAN ケーブルを抜く → 挿す → 再開 | 同上 |
| （任意）LAN アダプタ HIT-0300 | T1 は dcload-ip 任せ。T2 は KOS のドライバを確認 | **実機**：動くこと、速度の記録 |

G2 の判定基準（`PLAN.md`）は G2-5a で満たす。G2-5b は「誰でも使える形」の達成。

---

## 10. リスク

| # | リスク | 影響 | 手当て |
|---|--------|------|--------|
| R1 | SD への書き込みが遅い（未計測。CMD25 の PIO 送り） | 1 GB に 30 分以上かかる | G2-2b で最初に測る。F9〜F11 |
| R2 | G1 のビット化け（上流で報告あり、未再検証） | 書いたイメージや FAT が壊れる | F8 の照合と書き直し、読み戻し検証、メタデータにも照合 |
| R3 | KOS の `g1ata` と NetGD ファームの組み合わせが実機で未検証（DMA 書き込み、LBA48、SET FEATURES、タイムアウトなし） | アプリが固まる、書き込みが失敗する | G2-2a の実機項目。だめなら PIO に落とす。必要なら自前の ATA 書き込みを持つ |
| R4 | `ngfs_commit_ext` が FatFs の内部（`FIL.dir_sect` / `dir_ptr`）に依存する | FatFs の版を上げると壊れる | 版を固定して同梱。ホストテストで確定後のイメージを `fsck.fat` にかける |
| R5 | 大きなカードで `f_expand` の連続領域探しや空きの数え直しが遅い（FAT が数十 MB） | `BEGIN` に数十秒〜数分 | 進捗を表示。FSInfo を使う。実機で時間を測り、必要なら空き領域の探し方を自前にする |
| R6 | T1 の noexec（PC 側の異常でアプリが dcload の中で止まる） | 電源の入れ直しが要る | 再開できる設計で実害を抑える。T2 で解消 |
| R7 | KOS の TCP が SD 書き込みより遅い | T2 が T1 より遅くなる | G2-3c で測る。遅ければ受信側のバッファと ACK の間隔を調整。それでも遅ければ UDP の独自転送を検討（今は設計しない） |
| R8 | ASCII 以外のフォルダ名がマウントできない（`MOUNT_NAME` が 1 バイト 1 文字） | 日本語タイトルのフォルダが使えない | 当面は PC ツールが ASCII に限る。ファームの名前処理を UTF-8 にするかは G3 で判断 |
| R9 | 99 トラック級の GDI はエクステントが 96 本を超える（1 トラック 1 本でも足りない） | 一部のトラックが遅い経路になる | 書き込み側では解決できない。`MAX_EXTENTS` の拡大を G3-4 に申し送り |
| R10 | 排出・再マウントを BIOS や KOS の GD-ROM ドライバがどう受け取るか未検証 | 終了後に BIOS がディスクを認識しない | G2-3b で確認。必要なら終了時に `cdrom_reinit` 相当を呼ぶ、または常に BIOS へ戻る |

---

## 11. ユーザーへの質問

1. **SD 上のフォルダの決まり**：NetGD のファームは「フォルダ名」でマウントし、既定値は `GDEMU.CFG` で決める。一方、本家 GDEMU と GDmenu は `01` からの連番と `GDEMU.ini`。送信先の既定は「イメージ名のフォルダ」でよいか。それとも連番フォルダを自動で振る方式（GDmenu との互換）が要るか。使う予定のメニューソフトは何か。
2. **最初の実機検証は T1（dcload-ip 経由）だけで進めてよいか。** T2（単体 TCP）は G2-5a のあとに着手する順でよいか。
3. **Dreamcast-Dev-Tools-for-BBA のプリセット**：リポジトリの外にある ELF を指定できるか。ビルドコマンドと `PostRun` に引数（イメージのパスなど）を渡す方法はあるか。無ければ、プリセットに項目を足すのと、`netgd-send` 側で設定ファイルを読むのと、どちらがよいか。
4. **本体のリセット**：転送中に A+B+X+Y+Start やリセットボタンを押したとき、G1 の RSTn が動くかどうかを UART ログ（`host reset seen`）で確認してほしい。F5 の作りがこれで決まる。
5. **読み戻し検証の既定**：安全側（常に読み戻す。時間はおよそ 1.5 倍）で始めて、G1 のビット化けが出ないと分かったら外す、という方針でよいか。
6. **`NETGD.MF` を確定後も残してよいか。** 残すと後から検証できるが、イメージのフォルダに見慣れないファイルが 1 つ増える。
7. **対象のカード**：FAT32 だけに対応する（exFAT は非対応、フォーマット機能なし）でよいか。使っているカードの容量とクラスタサイズは。
8. **日本語のフォルダ名**が必要か（R8）。必要ならファームの名前処理の変更が G2 の範囲に入る。
9. **LAN アダプタ HIT-0300** の実機はあるか。無ければ「T1 では dcload-ip が対応しているので動くはず」という扱いにとどめる。
10. **アプリの配布形態**：T2 用に `netgd-recv` の CDI を作る手順（`mkdcdisc` など）を、ユーザーの `make_dcload_disc.ps1` と同じ流儀でそろえるか。

---

## 12. 出典

- このリポジトリ：`mcu/opengdemu/src/{ide,gdi,fat,cdi,button,sdcard,fwupdate}.rs`、`mcu/embassy-atsam3/src/hsmci.rs`、`mcu/fwupdater/src/main.rs`、`fpga/amaranth/hw/ide_device.py`、`docs/AddressMapMCU.md`、`docs/PLAN.md`、`README.md`
- KallistiOS（master）：`kernel/arch/dreamcast/hardware/g1ata.c`、`addons/libkosfat/fs_fat.c`（https://github.com/KallistiOS/KallistiOS）
- Dreamcast-Dev-Tools-for-BBA：`KNOWHOW_ja.md` 2〜7・10・13 章、`README.md`
- dcload-ip（https://github.com/sizious/dcload-ip）：BBA（HIT-0400）と LAN アダプタ（HIT-0300）の両対応
- FatFs（http://elm-chan.org/fsw/ff/）：`f_expand`、`FIL` 構造体。**版ごとの内部構造は実装時に同梱版で確認すること**
