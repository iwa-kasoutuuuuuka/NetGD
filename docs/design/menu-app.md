# 設計書：NetGD 専用メニュー `netgd-menu`

担当：Fable 5.2（設計）。実装は Sonnet、レビューと統合は Opus。
状態：**設計のみ。コードは 1 行も無い。実機では何も確かめていない。**
この文書の KOS の API 名は KallistiOS 2.x の記憶に基づく。**KOS 2.3.0（ユーザーのフォーク）のヘッダでの確認は実装時に行う**（§10 の一覧）。

根拠の記号：**[コード]** このリポジトリのコードを読んで確認／**[設計]** 既存の設計書・LOG／**[推定]** 未確認。

---

## 1. 目的と範囲

SD カードのフォルダ `01` に置く Dreamcast 用メニュー（GDmenu / openMenu の位置づけ）。NetGD のファームにだけ対応する。

### 1.1 目標

| # | 目標 |
|---|------|
| M-1 | SD カードのゲームを一覧し、選んで起動する。切替は既存の `0x52`（番号指定）を使う |
| M-2 | 一覧は **メニュー自身が SD を読んで作る**。PC 側のカード管理ツールで一覧を焼き込む必要が無い |
| M-3 | BBA があれば、メニューの画面から **PC → SD のイメージ書き込み（G2）** ができる。dcload-ip は要らない |
| M-4 | BBA が無くても M-1・M-2 はそのまま動く |
| M-5 | 中核（一覧、状態機械、プロトコル）は PC の gcc でビルドして試験できる。KOS 専用の層は薄くする |

### 1.2 やらないこと（v1）

- 純正 GDEMU への対応（`0xE0` VERSION が `OGDM` を返さなければ、SD に触らず案内だけ出す）。
- メニューからの `GDEMU.INI`・`GDEMU.CFG` の書き換え（読むだけ）。
- SD のフォーマット、FAT32 以外のカード。
- `GAME.CFG` の `type = "elf"`（プログラムの直接起動）、`next`（ディスクの連結）の表示。
- 3D 表示（PVR）、音、日本語のタイトル表示（ASCII 以外は `?` に置き換える）。
- メニュー自身による IP.BIN + 1ST_READ.BIN の直接起動（§5.4。実機の結果を見てから別の段階で検討）。
- T1（dcload-ip の `/pc/`）。T1 は今までどおり `tools/netgd-recv` を使う。

---

## 2. 主な判断（採用と却下）

| # | 判断 | 理由 | 却下した案 |
|---|------|------|-----------|
| D1 | 一覧は **アプリが SD を読む**（セカンダリ IDE ディスク + FatFs、読み取り専用） | ファームは一覧を持たないと決めて実装済み（[コード] `ide.rs` `handle_image_ctl` のコメント「Enumerating the card is the menu's job」、VERSION の件数欄は予約 0）。2E のフラッシュの残りは数百 B（PLAN）。書き込み用の FAT 層がすでにアプリ側にある | ファームが一覧を返すコマンドを足す（フラッシュと RAM が無い。プロトコル版も上がる）／openMenu 方式の `OPENMENU.INI` をメニューのイメージに焼き込む（BBA で足したゲームが PC のツール無しでは出ない。M-2 に反する） |
| D2 | 一覧の読み取りは **書き込みセッションなし・イメージをマウントしたまま** | セカンダリの読み出しはメディアがあっても通る（[コード] `handle_disk_read` に `media_present` の検査は無い。書き込みだけ ABRT）。起動のたびに排出するとリセット時の復帰（F5）に頼る場面が増える | 起動時に毎回セッションを開く（書き込み可能な窓が毎回開く） |
| D3 | 一覧用のマウントは **FAT の全走査をしない**「軽いマウント」を足す | `ngfs_mount` は FAT 全体を 2 回読む（128 GB で 15〜60 秒の見積もり、[設計] netgd-recv README）。空き容量は一覧に要らない | `ngfs_mount` をそのまま使う（起動のたびに数十秒） |
| D4 | 起動は **`52 82 lo hi` → READY 待ち → BIOS に戻す**（既定は BIOS メニュー、設定で再起動） | BIOS がディスクの検査・スクランブル解除・起動を全部行うので、ディスクの種類ごとの差をアプリが持たない。ローダーのコードが要らない | openMenu と同じ自前ローダー（`gdmenu_loader`。出所とライセンスの確認、機種・ディスク種別ごとの差、実機でしか試せないコードが増える。§5.4 に後の段階として残す） |
| D5 | G2 の取り込みは **T2（単体 TCP）を新しく実装**し、受信コア `recv_core.c` はそのまま使う | SD から起動したメニューに `/pc/` は無い。T2 は G2 設計（`bba-sd-write.md` 5.4）の第 2 段で、未実装 | メニューから dcload-ip のイメージに切り替えて `netgd-recv` を使う（今と同じ手順で、統合にならない）／`xport_pc.c` を流用（`/pc/` が無い） |
| D6 | T2 のデータは **Dreamcast が要求し PC が答える**（`WANT` → `DATA`、64 KiB ごと） | `ng_put` は「読む側が呼ぶ」形（`ng_src_t.read`）で、ホスト試験 29 シナリオが通っている。プロトコルをそれに合わせれば受信コアを変えずに済む。流れの同期ずれが起きない | `bba-sd-write.md` 5.4 の「PC が流し続け、DC は ACK」方式（`ng_put` を確保／転送／確定に割る必要がある。2 スレッドとリングも要る。速度が足りないと分かってから検討） |
| D7 | 進捗と結果は **既存の `NGD:` 行をそのままフレームに入れて返す** | `netgd-send` の `parse_result`（`job.py`）がそのまま使える。行の種類を二重に定義しない | 5.4 の `*_OK` フレームを種類ごとに定義 |
| D8 | netgd-recv のソースは **移動せず、Makefile から相対パスで参照**。オブジェクトは `tools/netgd-menu/build/` に出す | netgd-recv のビルドとテスト（R62）を壊さない。`.o` を元の場所に出すと、フラグの違う 2 つのビルドが同じ `.o` を取り合う | `tools/netgd-common/` に移す（差分が大きく、R62 の検証をやり直すことになる）／コピー（二重管理） |
| D9 | 画面は **文字のマス目（53 × 20）** を中核が作り、KOS 層は `bfont` で描くだけ | 画面の中身をホスト試験で文字として比べられる。netgd-recv・netgd-fwupd と同じ描き方 | PVR で描く（試験できない KOS のコードが増える） |
| D10 | 画面の文言は v1 では **英語（ASCII）**。文言は ID の表にまとめる | `bfont` の日本語は EUC-JP の変換表が要る（netgd-fwupd は `gen_msgs.py` で生成）。まず動くものを小さく | 最初から日英 2 言語（後から表に列を足せば入る） |
| D11 | ファーム更新の統合は **最後の任意の段階**。それまでは `netgd-fwupd` を使う | 失敗すると SWD でしか戻せない（PLAN「自己更新」）。対策 B が実機で動く前に入口を増やさない | 最初からメニューに入れる |
| D12 | ネットワークは **取り込み画面に入ったときに初めて初期化**（`INIT_NET` を付けない） | BBA が無い本体で起動が遅れたり止まったりしない（M-4） | `KOS_INIT_FLAGS` に `INIT_NET` |

---

## 3. 使い方の流れ

### 3.1 ゲームを起動する

1. 電源を入れる。ファームがフォルダ `01`（または `GDEMU.CFG` の `image=`）をマウントし、BIOS がメニューを起動する。
2. メニューが SD のルートを読み、番号フォルダを番号順に並べる。タイトルは後から埋まっていく（§6.2）。
3. 十字キーで選び、A で起動。メニューは `52 82 lo hi` を送り、READY を待ち、BIOS に戻る。
4. 既定（BIOS メニューに戻る方式）では、BIOS の画面で「プレイ」を選ぶ。再起動方式では起動画面のあと自動で始まる。
5. ゲーム中にリセットしたときの戻り先は `GDEMU.INI` の `reset_goto` で決まる。メニューは値を表示し、0 のときは「リセットではメニューに戻らない」と 1 行出す。

### 3.2 PC からイメージを書き込む（BBA あり）

1. 一覧画面で Y。メニューが BBA を初期化し、IP アドレス・ポート・4 桁の合言葉を表示して待つ。
2. PC で `netgd-send push game.gdi --host <IP> --code <合言葉> --slot auto`。
3. メニューが書き込みセッションを開く（メニューのイメージは排出される。メニューは RAM 上で動き続ける）。FAT の走査 → 確保 → 転送 → 検証 → 確定。画面に進捗、PC にも同じ行が返る。
4. PC が `finish` を送るか、利用者が B で取り込み画面を出ると、セッションを閉じて一覧を読み直す。新しいゲームが一覧に出る。
5. 途中で止まった（電源断、LAN 断、B）ときは、同じ `push` をもう一度（`--mode resume` が既定）。

### 3.3 BBA が無い

Y を押すと「BBA not found」と出て一覧に戻る。それ以外は 3.1 と同じ。

### 3.4 dcload-ip から起動した（開発時）

一覧と起動は動く。取り込みは無効（dcload が BBA を使っているため。「Started from dcload: use netgd-recv」と出す）。

---

## 4. 構成

### 4.1 層

```
┌──────────────── KOS 専用（薄い層、ホストでは試験できない） ────────────────┐
│ main_kos.c  plat_kos.c  net_kos.c        ＋ netgd-recv の dev_kos.c  fwxfer_kos.c │
└───────────────────────────────────────────────────────────┘
        ↑ nm_plat.h / nm_net.h / ngdev.h / fwxfer.h（この 4 つの境界だけ）
┌──────────────── 中核（C11、ホストでビルド・試験する） ────────────────────┐
│ nm_ui  nm_screen  nm_input  nm_list  nm_scan  nm_launch  nm_cfgparse  nm_ipbin   │
│ ngtp_frame  ngtp_srv                                                              │
│ netgd-recv: recv_core  ngfs  ngblk  diskio  manifest  crc32  fwctl  ＋ FatFs      │
└───────────────────────────────────────────────────────────┘
        ↓ ホスト試験では
  test/plat_host.c  test/net_host.c  ＋ netgd-recv の test/dev_file.c  test/fwctl_sim.c
```

### 4.2 作るファイル（`tools/netgd-menu/`）

| ファイル | 内容 | ホスト |
|----------|------|:--:|
| `Makefile` | KOS 用。`include $(KOS_BASE)/Makefile.rules` の行を必ず持つ（`tools/ci/kos-build.sh` がこの行で対象を選ぶ）。既定のターゲットは ELF だけ。`make cdi` は別（§9） | — |
| `Makefile.host` | gcc 用。`build/menu_host(.exe)` を作る | ○ |
| `build.ps1` | DreamSDK から `make` を呼ぶ（`tools/netgd-recv/build.ps1` と同じ形） | — |
| `ip.txt` | `makeip` 用のテンプレート（製品名 `NETGD MENU`） | — |
| `README_ja.md` | 使い方、SD への置き方、「実機未検証」の明記、初回確認リスト | — |
| `src/nm.h` | 版、上限値、`nm_game_t`、エラー値 | ○ |
| `src/nm_text.c/.h` | 文字列の整形（ASCII 以外を `?`、前後の空白を削る、連続する空白を 1 つに、幅に合わせて `..` で切る） | ○ |
| `src/nm_slot.c/.h` | 番号フォルダの規則（§6.1）。`common/discimg/src/slot.rs` の `number_of` と同じ結果 | ○ |
| `src/nm_cfgparse.c/.h` | `GDEMU.INI`（`reset_goto`・`open_time`・`detect_time`）、`GAME.CFG` の `title`、`name.txt` の 1 行目 | ○ |
| `src/nm_ipbin.c/.h` | `.gdi` からトラック 3 のファイル名・セクタ長を取る。IP.BIN の先頭 256 バイトからタイトルを取る | ○ |
| `src/nm_list.c/.h` | 一覧（固定長の配列、並べ替え、検索） | ○ |
| `src/nm_scan.c/.h` | SD の走査（FatFs、1 回の呼び出しで 1 フォルダ進む） | ○ |
| `src/nm_launch.c/.h` | 起動の手順（マウント → READY 待ち）。BIOS へ渡すのは `nm_plat_handoff` | ○ |
| `src/nm_input.c/.h` | ボタンの立ち上がりとオートリピート | ○ |
| `src/nm_screen.c/.h` | 53 × 20 の文字と属性のマス目、行の描画、進捗バー | ○ |
| `src/nm_ui.c/.h` | 状態機械と各画面（§7） | ○ |
| `src/ngtp.h`・`src/ngtp_frame.c` | NGTP のフレームの組み立てと検査（§8.2） | ○ |
| `src/ngtp_srv.c/.h` | NGTP のサーバ。操作を `recv_core` に渡す。`ng_sink_t`・`ng_src_t` の実装 | ○ |
| `src/nm_plat.h`・`src/nm_net.h` | 境界（§4.4） | ○ |
| `src/main_kos.c` | 起動、主ループ、マス目を `bfont` で描く、maple の読み取り | KOS |
| `src/plat_kos.c` | `nm_plat_*` と、受信コアが要る `ng_plat_*`・`get_fattime` | KOS |
| `src/net_kos.c` | `nm_net_*`（`net_init`、ソケット） | KOS |
| `test/menu_host.c` | ホスト試験の入口（§11.1） | ○ |
| `test/plat_host.c`・`test/net_host.c` | 境界のホスト実装（時計は仮想、ソケットは 127.0.0.1。`_WIN32` で Winsock） | ○ |
| `test/run_tests.py` | `unittest`。`tools/netgd-recv/test/run_tests.py` と同じ作法（作業フォルダは一時フォルダ、子プロセスに時間制限） | ○ |

### 4.3 `tools/netgd-recv` から使うもの

変更なしで使う：`crc32.c` `manifest.c` `diskio.c` `recv_core.c` `dev_kos.c` `fwxfer_kos.c` `third_party/fatfs/*`、試験用の `test/dev_file.c` `test/simdisk.h` `test/fatcheck.py`、`test_host`（カードのイメージ作成：`mkimage`・`addfile`・`mkdirs`・`dump`）。

**追加する（既存の動作は変えない。手順 1）：**

| ファイル | 追加 |
|----------|------|
| `src/ngdev.h`・`src/ngblk.c` | `extern int ngblk_readonly;`。1 のとき `ngblk_write` はデバイスに触らず `-1` を返す |
| `src/ngfs.h`・`src/ngfs.c` | `ng_err_t ngfs_mount_light(char *why, size_t whyn);`：`ngfs_mount` と同じ検査（セクタ 0 の署名、第 1 パーティションの種別 0x0B/0x0C、`f_mount`、FAT32 かつ `volbase` が一致）をして、**`scan_fat` を呼ばない**。成功したら `ngblk_readonly = 1`。`ngfs_mount`（全走査の方）は入口で `ngblk_readonly = 0`。`ngfs_unmount` は `ngblk_readonly` が 1 なら `ngdev_flush` を呼ばず、最後に 0 に戻す。軽いマウントの間に使ってよいのは `f_opendir`・`f_readdir`・`f_closedir`・`f_open(FA_READ)`・`f_read`・`f_lseek`・`f_close`・`f_stat` と `ngfs_default_image`・`ngfs_classify` だけ |
| `src/fwctl.h`・`src/fwctl.c` | §5.2 の 6 関数 |
| `test/fwctl_sim.c`・`test/simdisk.h` | `0x52`（82・81・00）、`0x00`（TEST UNIT READY）、`E0 02`（名前あり）の模型。変数：`sim_slots[]`（存在する番号）、`sim_mounted_number`、`sim_notready_polls`（マウント後に NOT READY を返す回数）、`sim_gdemu_mounts` |

使わない：`main.c` `xport_pc.c` `plat_kos.c`。`recv_core.c` が `xport_pc.c` のシンボルを参照していないことは、手順 2 のホストビルドのリンクで確かめる（参照していたら Opus に報告。勝手に受信コアを変えない）。

### 4.4 境界

`nm_plat.h`（KOS：`plat_kos.c`、ホスト：`test/plat_host.c`）

| 関数 | 意味 |
|------|------|
| `uint32_t nm_plat_millis(void)` | 起動からの ms |
| `void nm_plat_sleep(uint32_t ms)` | 待つ（ホストは仮想時計を進めるだけ） |
| `uint32_t nm_plat_buttons(void)` | いま押されているボタンのビット（`NM_BTN_UP/DOWN/LEFT/RIGHT/A/B/X/Y/START/LT/RT`）。アナログは ±64 を超えたら十字キー扱い、トリガーは 128 を超えたら押下 |
| `void nm_plat_present(const nm_screen_t *s)` | マス目を画面に出す（前回と違うマスだけ描く） |
| `void nm_plat_handoff(int mode)` | `NM_HANDOFF_BIOS`：BIOS メニューへ／`NM_HANDOFF_REBOOT`：再起動。KOS では戻らない。ホストは `HANDOFF bios` を出力して終了 |
| `int nm_plat_dcload(void)` | dcload-ip から起動したなら 1 |
| `uint32_t nm_plat_entropy(void)` | 合言葉の種（KOS：タイマのカウンタと RTC。ホスト：`--code` で固定） |

`nm_net.h`（KOS：`net_kos.c`、ホスト：`test/net_host.c`）。接続は同時に 1 本。

| 関数 | 戻り値 |
|------|--------|
| `int nm_net_up(char *ip, size_t n)` | 0／`NM_NET_NODEV`（BBA なし）／`NM_NET_NOADDR`（アドレスが取れない） |
| `int nm_net_listen(uint16_t port)` | 0／-1 |
| `int nm_net_accept(uint32_t timeout_ms)` | 1 接続／0 時間切れ／-1 |
| `int nm_net_recv_all(void *buf, uint32_t n, uint32_t timeout_ms)` | 0／-1（切断・時間切れ） |
| `int nm_net_send_all(const void *buf, uint32_t n)` | 0／-1 |
| `void nm_net_close_client(void)`・`void nm_net_stop(void)` | 接続を閉じる／待ち受けをやめる（BBA は初期化したまま） |

受信コアが要る `ng_plat_*`（`ng_plat.h`）と `get_fattime` は、メニュー側の `plat_kos.c`／`plat_host.c` が定義する。`ng_plat_user_stop` は「取り込み中に B が押された」フラグを返す。

---

## 5. ファームとのやりとり

### 5.1 送り方

- プライマリ（GD-ROM）への ATAPI PACKET：`fw_xfer`（`tools/netgd-recv/src/fwxfer_kos.c`。G1 のレジスタを直接、`g1_ata_mutex_lock` の中で）。12 バイトの CDB、未使用のバイトは 0。
- センスキーは `fw_xfer` の `*err >> 4`。6（UNIT ATTENTION）は「実行されなかった」ので 1 回だけ再送（`fwctl.c` の `atapi()` が行う）。
- セカンダリ（SD）：KOS `g1ata`（`dev_kos.c`）。
- **KOS の cdrom ドライバは使わない**（`KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)`、netgd-recv と同じ）。`/cd/` は読まない。要る物は全部 ELF に入れる（v1 は外部ファイルなし）。

### 5.2 使うコマンド（すべて既存。ファームの変更は無い）

| 関数（`fwctl.c` に足す。○は既存） | CDB | 応答 | 根拠 |
|-----------------------------------|-----|------|------|
| ○ `fw_version` | `E0 03 00 00 20` | 32 バイト：`"OGDM"`、版、機能ビット、3 バイト、版の文字列 24 | [コード] `ide.rs` `SUB_VERSION` |
| `int fw_gdemu_mount(uint16_t n, uint8_t *sense)` | `52 82 <n 下位> <n 上位>` | なし。番号のフォルダが無い・イメージが無い・マウント失敗はセンスキー 5（いまのメディアはそのまま）。`n = 0` は既定に戻す | [コード] `handle_gdemu_cmd` |
| `int fw_gdemu_step(int prev, uint8_t *sense)` | `52 81 55`（次）／`52 81 44`（前） | なし | 同上 |
| `int fw_gdemu_version(uint32_t *boot, uint32_t *fw)` | `52 00 00 00 08` | 8 バイト：LE の u32 が 2 つ（ブートローダ、ファーム）。`major<<24 \| minor<<16 \| patch<<8`、各部 BCD。5.20.0 は `00 00 20 05` | 同上 |
| `int fw_mount_named(const char *name, uint8_t *sense)` | `E0 05 <chunk> <名前 9 バイトまで>` を必要な回数（既存の `stage_name`）→ `E0 02 00` | なし。失敗はセンスキー 5 | [コード] `SUB_MOUNT_NAME`・`SUB_MOUNT` |
| `int fw_mount_default(uint8_t *sense)` | `E0 02 FF` | なし | 同上 |
| `int fw_test_unit_ready(uint8_t *sense)` | `00` | なし。0 = READY。-1 のとき `*sense` が 2（NOT READY）か 6（UNIT ATTENTION）。**この関数だけは 6 でも再送しない**（呼ぶ側が回数を数える） | [コード] `ATAPI_TEST_UNIT_READY` |
| ○ `fw_session_begin`・`fw_session_end`・`fw_status` | `E0 06 'N' 'G' 'W' 'S'`／`E0 07 <0\|1\|2>`／`E0 08 00 00 30` | 取り込みのときだけ（`recv_core.c` が呼ぶ） | [コード] `wsession.rs` |

戻り値はどれも 0 = 成功、-1 = 失敗。`sense` は NULL 可。

ファームの決まり（[コード]）：書き込みセッション中は `52 82`・`52 81`・`E0 02` を拒否（センスキー 5）。セカンダリへの書き込みはメディアがある間 ABRT。セッション中は MBR とパーティションの手前への書き込みを拒否（F7）。`0xF9` はセッション中は拒否。

### 5.3 起動の手順（`nm_launch.c`）

1. 前提を作る：書き込みセッションが開いていれば閉じる（`ng_session_close(sk, 1, NULL, 1)`）。軽いマウントは `ngfs_unmount`。
2. マウント：番号のある項目は `fw_gdemu_mount(n)`。番号の無い項目（ルート直下の名前フォルダ）は `fw_mount_named(folder)`。
   - センスキー 5 → `NM_L_NOIMAGE`（「No image in folder NN」。一覧に戻る。メニューのイメージはマウントされたまま）。
   - それ以外の失敗 → `NM_L_FWERR`。
3. READY 待ち：`fw_test_unit_ready` を 20 ms ごと。上限は `10000 + open_time + detect_time` ms（INI に書かれたキーの分だけ足す。書かれていなければ 10000）。0 が **2 回続けて**返ったら READY。
   - 時間切れ → `NM_L_NOTREADY`（「Drive not ready」。A でもう一度待つ、B で一覧へ。この時点でマウントされているのは選んだゲーム）。
4. 画面に「Starting NN ...」を出して `nm_plat_present`、100 ms 待ち、`nm_plat_handoff(mode)`。

`nm_launch` は 1 回の呼び出しで 1 歩だけ進む形（`nm_launch_step()` が `NM_L_BUSY`／結果を返す）にして、主ループが止まらないようにする。

### 5.4 BIOS への渡し方

| 方式 | KOS | 利点 | 懸念 |
|------|-----|------|------|
| `NM_HANDOFF_BIOS`（**既定**） | `arch_menu()` | リセットを伴わない。`reset_goto` に触れない | BIOS の画面で「プレイ」を押す一手間。BIOS がメディアの入れ替わり（UNIT ATTENTION）を拾うかは実機で確認（§14 H2） |
| `NM_HANDOFF_REBOOT` | `arch_reboot()` | 自動で起動する | 起動画面の時間。BIOS の再起動が G1 のハードリセットを出すなら、`reset_goto=1` のカードでは **メニューに戻ってしまい起動できない**（[コード] `ide.rs` の hard reset の経路は `reset_goto` を見る。SRST では動かない）。実機で確認（§14 H3） |
| 直接起動（v1 では作らない） | — | 最短 | openMenu の `gdmenu_loader` 相当が要る。出所・ライセンス・MIL-CD のスクランブル解除・ハードの後始末を調べてから、別の設計にする |

既定は実機の結果（H2・H3）で決め直す。方式は情報画面（§7.2 の `INFO`）で切り替えられ、ビルド時の既定は `-DNM_HANDOFF_DEFAULT=`。

---

## 6. 一覧

### 6.1 何を一覧に出すか

- **番号フォルダ**：ルート直下で、名前が ASCII 数字 2〜4 桁、値が 1 以上（`01`、`002`、`1000`。[コード] `slot.rs` `number_of`）。ファームは 8.3 名の完全一致で番号を判定するので（R14）、`02 Title` のような名前は番号フォルダではない。
  - 同じ番号が 2 つ（`02` と `002`）あるときは、**桁数の少ない方**だけを出す。ファームがどちらを選ぶかは未確認（§14 H8）なので、状態行に「duplicate number NN」と出す。
- **名前フォルダ**：ルート直下で番号フォルダでなく、完成したイメージがあるもの（`netgd-send --name` で書いたもの）。番号 0 として持ち、`fw_mount_named` で起動する。2 階層目以降は v1 では出さない。名前が `ng_path_ok` を通らない（ASCII 以外など）フォルダは出さない。
- **フォルダ `01`** は出さない（メニュー自身という慣習。`-DNM_SHOW_SLOT1=1` で出す）。
- 並び：番号フォルダを番号の昇順、その後に名前フォルダを ASCII の昇順（大文字小文字を区別しない）。
- 上限 `NM_MAX_GAMES = 1024`。超えた分は捨て、状態行に「list truncated」。

項目 `nm_game_t`：`number`（u16、0 = 名前フォルダ）、`folder[97]`、`title[65]`、`kind`（不明／GDI／CDI）、`state`（未走査／完成／途中／イメージなし）、`flags`（既定のイメージ = `GDEMU.CFG` の `image=`、番号の重複）。

### 6.2 走査（`nm_scan.c`）

軽いマウント（D3）の上で、**1 回の `nm_scan_step()` で 1 フォルダ**。主ループは 1 フレームに 1 回呼ぶ。走査中も操作できる。

1. **ルートの 1 周**：`f_opendir("0:/")` → 全エントリ。ディレクトリだけを拾って仮の項目を作る（状態 = 未走査、タイトル = フォルダ名）。同時に `GDEMU.INI` があれば読む（§6.4）。`ngfs_default_image` で既定のイメージを得る。この 1 周は分割しない（ルートのエントリ数は多くても数百）。終わったら並べ替えて一覧を表示。
2. **フォルダごと**（番号順）：`f_opendir` → 全エントリを 1 回見る。
   - 拡張子 `GDI`／`CDI` のファイルがある → 完成。`NGP` か `NETGD.CKP` がある（かつ GDI/CDI が無い）→ 途中。どれも無い → イメージなし。
   - `GAME.CFG`・`NAME.TXT`（大文字小文字を区別しない）の有無と、`.gdi` のファイル名を覚える。
   - タイトルを決める（§6.3）。
   - イメージなしの名前フォルダは一覧から外す。イメージなしの番号フォルダは「(empty)」として残し、起動はできない表示にする。
3. 全フォルダが済んだら走査完了。状態行に件数。

エラー：`f_opendir`・`f_read` が失敗したフォルダは状態「read error」にして次へ進む。3 フォルダ続けて失敗したら走査を止めて状態行に「SD read error」（一覧は残す。起動は番号だけで試せる）。

走査が終わっていない項目でも A で起動を試せる（番号があれば足りる。イメージが無ければファームがセンスキー 5 を返す）。

時間の見積もり（**未計測**）：1 フォルダあたり 1 セクタ読みが 5〜10 回（`ngblk_read` は 1 セクタを 2 回読む）。100 本で 1,000〜2,000 コマンド。1 コマンドの時間は実機で測る（§14 H5）。長ければキャッシュ（§13 の後の段階）を検討。

### 6.3 タイトルの決め方（先に見つかったもの）

1. `GAME.CFG` の `title = "..."`（[コード] `common/gamecfg`。最大 64 文字）。
2. `NAME.TXT` の 1 行目（GDMENU Card Manager が作る。[設計] R7。二次資料）。
3. GDI：`.gdi` を読み（4 KiB まで）、トラック 3 の行（3 行目のトラック。無ければ LBA 45000 の行）のファイル名とセクタ長（2048 か 2352）を取り、そのファイルの先頭 1 セクタを読む。2352 のときはユーザーデータが 16 バイト目から。先頭 16 バイトが `SEGA SEGAKATANA ` なら、オフセット 0x80 から 128 バイトをタイトルにする。引用符付きのファイル名と BOM（R65）にも対応する。
4. どれも無ければフォルダ名。

どの場合も `nm_text` で整形（ASCII 以外は `?`、空白の整理、64 文字まで）。CDI の IP.BIN は読まない（CDI の解析を C に移す必要がある。`netgd-send push` が `name.txt` を一緒に書くので、BBA で書いた CDI にはタイトルが付く。§8.4）。

### 6.4 `GDEMU.INI`・`GDEMU.CFG`

- 読むだけ。`GDEMU.INI`（ルート、4 KiB まで）から `reset_goto`（0〜9999、範囲外は 0）、`open_time`・`detect_time`（150〜5000 に丸める。書かれていたかどうかも持つ）。規則は `common/gamecfg/src/ini.rs` と同じ（キーは大文字小文字を区別しない、`key=value`、知らないキーは無視）。Rust 側のテストの入力と期待値をホスト試験に写す。
- `reset_goto` の表示：0 →「Reset: stays in the game (reset_goto off)」。1 →「Reset: back to this menu」。それ以外 →「Reset: goes to NN」。
- `GDEMU.CFG` の `image=` のフォルダには一覧で `*` を付ける（受信コアが上書き・削除から守るフォルダ）。

---

## 7. 画面と状態機械

### 7.1 画面の割り付け

640×480、`bfont`（12×24）で 53 桁 × 20 行。マスは文字 1 バイトと属性（通常／反転／淡色／警告）。

| 行 | 内容 |
|----|------|
| 0 | `NetGD Menu <版>` と右端にファームの版（`fw 5.20.0`） |
| 2〜16 | 画面ごとの本体（一覧は 15 件） |
| 17 | 状態行（走査の進み、エラー、ヒント） |
| 18〜19 | ボタンの案内 |

一覧の 1 行：`>0002  Crazy Taxi                            GDI *`（カーソル、番号 4 桁か空白、タイトル 38 桁、種類、印）。途中のイメージは淡色で `(incomplete)`。

### 7.2 状態

```
BOOT ─┬─ NOFW（終端：START で BIOS へ）
      ├─ NOSD（同上）
      └─ LIST ⇄ INFO
          │ A                         Y
          ├→ LAUNCH ─ 成功 → HANDOFF  ├→ NET_START ─ 失敗 → LIST（メッセージ）
          │    └ 失敗 → DIALOG → LIST │      └→ RECV_IDLE ⇄ RECV_BUSY ⇄ CONFIRM
          │ START                     │            │ B（待機中）／FINISH／5 分無通信
          └→ HANDOFF（BIOS へ）       │            └→ RECV_CLOSE → LIST（走査し直し）
```

| 状態 | 入ったとき | 抜ける条件 |
|------|-----------|-----------|
| `BOOT` | 画面を出す → `fw_version` → `ngdev_init` → `ngfs_mount_light` → `fw_gdemu_version` | 失敗：`fw_version` → `NOFW`、`ngdev_init`／マウント → `NOSD`（理由を表示：no disk / not FAT32 / read error）。成功 → `LIST` |
| `NOFW`・`NOSD` | 説明を出す。SD にもファームにも何も送らない | START → `HANDOFF(BIOS)` |
| `LIST` | 走査を進めながら操作を受ける | A → `LAUNCH`、X → `INFO`、Y → `NET_START`、START → 確認の後 `HANDOFF(BIOS)` |
| `INFO` | ファームの版・機能ビット、`reset_goto` の表示、既定のイメージ、件数、渡し方（A で切り替え） | B → `LIST` |
| `LAUNCH` | §5.3 | 成功 → `HANDOFF`、失敗 → `DIALOG` |
| `DIALOG` | 1〜3 行のメッセージと選択肢 | A／B |
| `NET_START` | `nm_plat_dcload()` なら断る。`nm_net_up` → `nm_net_listen(20039)` → 合言葉を作る | 失敗 → `LIST`（「BBA not found」「No IP address」）。成功 → `RECV_IDLE` |
| `RECV_IDLE` | IP・ポート・合言葉・セッションの有無を表示。`nm_net_accept(0)` と接続中のフレーム待ちを毎フレーム | フレーム到着 → `RECV_BUSY`、B → `RECV_CLOSE`、5 分間フレームが無くセッションが開いている → `RECV_CLOSE` |
| `RECV_BUSY` | 操作 1 つを実行（`recv_core` の中にいる間、画面と入力は `ng_sink_t.ui`・`poll` から更新） | 終われば `RECV_IDLE`。B は「安全に止める」要求（`poll` が 1 を返す） |
| `CONFIRM` | PC が保護フォルダの上書き・削除（force）を求めた | A を 1 秒押し続けて許可、B か 30 秒で拒否 |
| `RECV_CLOSE` | セッションが開いていれば `ng_session_close(sk, 1, NULL, 0)`（`FINISH mount` を受けていたらその指定）→ `nm_net_close_client` → `nm_net_stop` → `ngfs_mount_light` → 走査をやり直す | → `LIST`。軽いマウントが失敗 → `NOSD` |

決まり：**軽いマウントと書き込みセッションは同時に存在しない。** `RECV_IDLE` に入るとき軽いマウントを外し、`RECV_CLOSE` で付け直す。

### 7.3 操作

| ボタン | 一覧 | 取り込み |
|--------|------|----------|
| 十字キー上下（アナログも） | 1 件移動。押し続けは 300 ms 後から 80 ms ごと | — |
| 十字キー左右、L／R | 1 ページ（15 件） | — |
| A | 起動 | （確認画面で）許可 |
| B | — | 待機中：出る／実行中：安全に止める |
| X | 情報 | — |
| Y | 取り込み画面へ | — |
| START | BIOS メニューへ（確認あり） | — |

コントローラはポート A〜D の最初に見つかった 1 台。無ければ状態行に「No controller」。

### 7.4 取り込み画面

行 2：`IP 192.168.0.20  port 20039  code 4821`。行 3：`waiting` / `client connected` / `session open`。行 5〜9：操作名とファイル番号、ファイル名、進捗バー、速度と量、残り時間（`netgd-recv` の `main.c` の `ui_progress` と同じ計算）。行 11〜15：最近の `NGD:` 行 5 本（`ERROR`・`DONE`・`FINISH`・`MOUNT pct=100`。間引き対象の行は出さない）。

---

## 8. 取り込み（NGTP 版 1）

`bba-sd-write.md` 5.4 を、D6・D7 のとおり変えたもの。**5.4 の表ではなくこの節が実装の基準。** 違いは、データの向き（`WANT`／`DATA`）、応答（`LINE`／`DONE`）、`BEGIN`・`COMMIT`・`ACK`・`FILE_END` を無くしたこと。

### 8.1 接続

- Dreamcast が TCP サーバ、ポート **20039**。同時に 1 本。接続中は次の接続を受け付けない。
- 探索（UDP 20039、`NGTP-DISCOVER 1` → `NGTP-HERE 1 name=... app=... state=...`）は任意の段階（手順 9）。それまでは画面の IP を `--host` に指定する。
- 認証も暗号化も無い LAN 用。合言葉は誤操作よけ：0000〜9999、取り込み画面に入るたびに作り直す。3 回間違えたら作り直す。

### 8.2 フレーム（リトルエンディアン。5.4 と同じ）

| オフセット | 長さ | 内容 |
|-----------|------|------|
| 0 | 4 | `"NGT1"` |
| 4 | 1 | 種別 |
| 5 | 1 | フラグ |
| 6 | 2 | 0 |
| 8 | 4 | 通し番号（PC が要求ごとに 1 増やす。Dreamcast の応答・`WANT` はその要求の番号、`DATA` も同じ番号） |
| 12 | 4 | ペイロード長 n（最大 65536 + 16） |
| 16 | n | ペイロード |
| 16+n | 4 | 先頭からここまでの CRC32（zlib） |

マジック違い・長さ超過・CRC 不一致・知らない種別は `ERROR` を返して切断。受信は「ヘッダ 16 バイト → ペイロード → CRC」の順に `nm_net_recv_all`。時間切れは、フレームの途中 15 秒、`WANT` に対する `DATA` 待ち 30 秒。

### 8.3 メッセージ

PC → Dreamcast（要求）：

| 種別 | 名前 | ペイロード | 動作 |
|------|------|-----------|------|
| 0x01 | `HELLO` | u16 版（1）、u16 合言葉、クライアント名（ASCII） | 最初に必ず 1 回。応答 `HELLO_OK`。版違い `ERROR EVERSION`、合言葉違い `ERROR EAUTH`（どちらも切断） |
| 0x02 | `STATFS` | なし | `ng_statfs` |
| 0x03 | `LIST` | フォルダ（`/` = ルート） | `ng_list` |
| 0x04 | `PUT` | u8 モード（0 new／1 resume／2 overwrite）、u8 フラグ（bit0 force、bit1 断片化を許す）、u16 0、タグ 4 バイト（ASCII 数字、`NGD:` 行の番号）、続けてマニフェスト全文（`bba-sd-write.md` 5.1） | `ng_manifest_parse` → `ng_put`。途中で `WANT` を出す |
| 0x05 | `DATA` | u16 ファイル番号、u16 0、u64 オフセット、データ。フラグ bit0 = 「PC が中止を求めている」 | `WANT` への答えとしてだけ有効 |
| 0x06 | `DELETE` | u8 フラグ（bit0 force）、フォルダ | `ng_delete` |
| 0x07 | `VERIFY` | フォルダ | `ng_verify` |
| 0x08 | `FINISH` | u8 指定（0 none／1 default／2 mount）、フォルダ | `ng_session_close(sk, 指定, フォルダ, 0)` → 取り込み画面を出て一覧へ（`RECV_CLOSE`）。応答を送ってから切断 |
| 0x09 | `PING` | なし | `PONG` |

Dreamcast → PC：

| 種別 | 名前 | ペイロード |
|------|------|-----------|
| 0x81 | `HELLO_OK` | u16 版、u16 フラグ（bit0 セッションが開いている）、u32 `DATA` の最大長（65536）、u8 OGDM の版、u8 機能ビット、ファームの版 24 バイト、アプリの版（ASCII） |
| 0x89 | `PONG` | なし |
| 0x90 | `LINE` | `NGD:` 行 1 本（改行なし）。フラグ bit0 = 間引いてよい行 |
| 0x91 | `DONE` | u8 成否（1 = 成功）、エラー名（ASCII。`ENOSPC` など `ng_err_name` の値。成功なら空） |
| 0x92 | `WANT` | u16 ファイル番号、u16 0、u64 オフセット、u32 長さ（最大 65536） |
| 0x7F | `ERROR` | エラー名、空白、説明（ASCII）。この後切断 |

規則：

- `HELLO` 以外の要求には、0 本以上の `LINE`、（`PUT` のときは `WANT`／`DATA` の往復）、最後に必ず 1 本の `DONE`。
- `HELLO` の前の要求、`WANT` を待っていないときの `DATA`、番号・オフセットの合わない `DATA` は `ERROR ESTATE` で切断。
- `DATA` のデータ長は `WANT` の長さ以下。短いのはファイルの末尾だけ。0 は末尾。
- **セッションは最初の `STATFS`／`LIST`／`PUT`／`DELETE`／`VERIFY` で開く**（`ng_session_open`。`NGD:MOUNT`・`NGD:HELLO` の行が `LINE` で流れる）。開けなければ `DONE` 失敗（`EFW`・`EFS`・`EIO`）。
- force（`PUT`・`DELETE` のフラグ bit0）は `CONFIRM` 画面で許可されたときだけ `ng_cfg.force = 1`。その操作が終わったら 0 に戻す。拒否なら force なしで実行する（保護フォルダなら `EPROTECTED`）。
- `ng_sink_t.poll` が 1 を返す条件：B が押された、または直前の `DATA` のフラグ bit0。受信コアは次の安全な点で止まり `NG_E_STOP`（チェックポイントは残る）。
- LAN が切れた・`DATA` が来ない：`ng_src_t.read` が負を返す → 受信コアは `ESRC` で終わる（チェックポイントは残る）→ 接続を閉じて `RECV_IDLE`。セッションは開いたまま（5 分で自動的に閉じる）。
- スレッドは 1 本。`RECV_BUSY` の間、画面の更新と B の読み取りは `ng_sink_t.ui`（とフレームの送受信の前後）から行う。

`ng_src_t` の実装（`ngtp_srv.c`）：`open(idx, off)` は位置を覚えるだけ。`read(buf, n)` は `WANT{idx, off, n}` を送り、`DATA` を受け、番号とオフセットを確かめ、データを `buf` に写して `off` を進める。`close` は何もしない。

### 8.4 PC 側（`tools/netgd-send` に足す）

| コマンド | 内容 |
|----------|------|
| `push <イメージ> --host IP --code NNNN [--slot N\|auto] [--name F] [--mode new\|resume\|overwrite] [--title T] [--mount] [--force] [--allow-fragmented] [--no-finish]` | 検査（`prepare` と同じ）→ マニフェスト → `HELLO` → `PUT` → `WANT` に答える → 結果を表示。既定のモードは `resume`。`--no-finish` が無ければ最後に `FINISH`（`--mount` なら mount、無ければ default） |
| `remote statfs\|list\|delete\|verify\|finish --host IP --code NNNN [引数]` | 単発の操作 |

- `prepare` の中の「検査して、SD 上のファイル名（8.3）と中身の対応を決める」部分を関数に切り出し、`prepare`（ジョブフォルダに置く）と `push`（元のファイルから直接読む。書き換えた `.gdi` はメモリ上）の両方から使う。`prepare` の出力は 1 バイトも変えない（既存の 42 テストが通ること）。
- `push` はマニフェストに **`name.txt`** を 1 ファイル足す（中身は `--title`、無ければ元のファイル名の拡張子を除いたもの + 改行。ASCII 以外は `?`）。イメージに元から `name.txt` があればそれを使う。
- `--slot auto`：先に `LIST /` を送り、`NGD:LIST` の行から使われている番号を集め、2 以上で最小の空き番号。フォルダ名は既存の `slot_folder(n)`。
- 終了コードは `collect` と同じ（0 成功／2 途中で再開可／1 失敗）。`NGD:` 行の解釈は `job.py` の `parse_result` を使う。
- 新しいモジュール：`netgd_send/ngtp.py`（フレーム、クライアント）。標準ライブラリだけ。

---

## 9. ビルド

### 9.1 CI（この PC に KOS は無い）

- `.github/workflows/kos.yaml` は `tools/*/Makefile` のうち `Makefile.rules` を含むものを全部ビルドする（[コード] `tools/ci/kos-build.sh`）。**`tools/netgd-menu/Makefile` を置けば、ワークフローを変えずに KOS 2.3.0 でコンパイルされ、`netgd-menu.elf` が成果物に入る。** `kos-build.sh` は `make clean` → `make HOST_DIR_REL=/ci` → `*.elf` をコピーするので、ELF は `tools/netgd-menu/` の直下に出すこと。
- `.github/workflows/build.yaml` の `tools-host` の表に `netgd-menu`（`python tools/netgd-menu/test/run_tests.py`）を 1 行足す。
- 実装者はこの PC で `Makefile.host` とホスト試験だけを回す。KOS 層のコンパイルの確認は、ブランチを push した後の `kos.yaml` の結果（Opus が見る）。KOS 層は小さく保つ：`main_kos.c`・`plat_kos.c`・`net_kos.c` の合計で 500 行以内を目安にし、判断を持たせない（判断は中核に置く）。

### 9.2 KOS 用 Makefile の決まり

- 自分のソース（`src/*.c`）のオブジェクトも、他のフォルダのソースのオブジェクトも **`build/kos/` に出す**（D8）。`../netgd-recv/src/%.c` → `build/kos/recv_%.o`、`../netgd-recv/third_party/fatfs/%.c` → `build/kos/ff_%.o` のように接頭辞で分け、明示のパターン規則で `kos-cc -c` を呼ぶ。`Makefile.rules` の既定の規則（`.o` をソースの隣に出す）に任せない。
- インクルードパス：`src`、`../netgd-recv/src`、`../netgd-recv/third_party/fatfs`。フラグは netgd-recv と同じ（`-std=gnu11 -Wall -Wno-unused-function -Wno-format-truncation`、`-O2`）。
- `-D` に引用符付きの文字列を渡さない（`kos-cc` を通る間に壊れる。[設計] netgd-recv README）。
- `clean` は `build/kos` と ELF を消す。

### 9.3 DreamSDK での手順（別の PC）

1. DreamSDK のシェルを開く（`KOS_BASE` が設定済み。KOS は 2.3.0 のユーザーフォーク）。
2. `cd tools/netgd-menu && make` → `netgd-menu.elf`。PowerShell からは `.\build.ps1`。
3. 開発中の実行：`dc-tool-ip -t <IP> -x netgd-menu.elf`（一覧と起動だけ。取り込みは無効、§3.4）。
4. ディスクイメージ：`make cdi` → `netgd-menu.cdi`。中身は次の順（ツール名は Makefile の変数にして差し替えられるようにする）：
   `sh-elf-objcopy -O binary` → `scramble`（KOS の utils）→ `1ST_READ.BIN`、`makeip ip.txt IP.BIN`（KOS の utils）、`mkisofs -C 0,11702 -V NETGD_MENU -G IP.BIN -l -o netgd-menu.iso <フォルダ>`、`cdi4dc netgd-menu.iso netgd-menu.cdi`。
   **`mkisofs`・`cdi4dc` が DreamSDK に入っているかは未確認**（無ければ `mkdcdisc` に替える）。CI では `make cdi` を実行しない。
5. SD に置く：フォルダ `01` に `netgd-menu.cdi` を入れる（他の `.gdi`／`.cdi` は置かない）。`GDEMU.INI` の `reset_goto=1` は H3 の結果が出るまで、渡し方を BIOS（既定）にしたときだけ使う。

---

## 10. KOS 2.3.0 で確認すること（実装時・CI・実機）

| # | 項目 | 確認の方法 |
|---|------|-----------|
| K1 | `arch_menu()`・`arch_reboot()`（`<arch/arch.h>`） | ヘッダ、CI |
| K2 | `net_init(uint32 ip)` を `INIT_NET` なしで後から呼べるか。BBA なしのとき `net_default_dev == NULL` で戻るか（待たされないか）。DHCP の待ち時間 | ヘッダ、CI、実機（BBA あり・なし） |
| K3 | ソケット：`socket`／`bind`／`listen`／`accept`／`recv`／`send`／`close`、非ブロッキング（`fcntl(O_NONBLOCK)`）か `poll()`。時間切れの付け方 | ヘッダ、CI、実機 |
| K4 | `net_default_dev->ip_addr` から表示用の IP を取る | ヘッダ、CI |
| K5 | `bfont_draw_str`・`vram_s`・`vid_set_mode(DM_640x480, PM_RGB565)`、`maple_enum_type(0, MAPLE_FUNC_CONTROLLER)`・`cont_state_t` | netgd-recv・netgd-fwupd が同じ物を使って CI を通っている（[設計] R87） |
| K6 | `dcload_type == DCLOAD_TYPE_IP` | netgd-recv が使用 |
| K7 | `INIT_CDROM` を外した KOS アプリを **GD-ROM（イメージ）から起動**して問題ないか | 実機（H1） |

---

## 11. ホスト試験

### 11.1 入口 `menu_host`

| コマンド | 内容 |
|----------|------|
| `menu_host selftest` | 純粋な関数の単体試験。失敗数を返す |
| `menu_host ui <img> <sectors> --script "<イベント>" [--slots 2,3,5] [模型のオプション]` | UI の中核を仮想時計で動かす。イベント：`U D L R A B X Y S LT RT`（1 回押して離す）、`hold:A:1200`（押し続け）、`t:500`（500 ms 進める）、`scan`（走査が終わるまで進める）、`dump`（画面 20 行を出力）。ファームへのパケットは `FW 52 82 03 00 ...`、BIOS への渡しは `HANDOFF bios` の行で出力 |
| `menu_host serve <img> <sectors> --port P --code C [--cut N] [--idle-ms N] [...]` | 取り込み画面の状態で 127.0.0.1 の TCP を待つ。`FINISH` か時間切れで終了。`--cut` は netgd-recv の電源断の模擬と同じ |

カードのイメージは `tools/netgd-recv` の `test_host mkimage`／`addfile`／`mkdirs` で作る（`run_tests.py` が両方の実行ファイルをビルドする）。模型のオプション（`--ua`・`--rflip` など）は netgd-recv の `test_host` と同じ名前にする。

### 11.2 シナリオ

**単体（selftest）**

| # | 対象 | 確かめること |
|---|------|-------------|
| U1 | `nm_slot` | `01`→1、`002`→2、`1000`→1000、`0`・`00`・`1`・`12345`・`0a`・`02 X` は番号でない（`slot.rs` のテストと同じ入力） |
| U2 | `nm_cfgparse` INI | `ini.rs` のテストの入力と期待値（既定値、丸め、大文字小文字、`reset_goto` 9999 超、キーが書かれたかどうか） |
| U3 | `nm_cfgparse` GAME.CFG／name.txt | 引用符付きの `title`、`#` コメント、`[section]` を飛ばす、64 文字で切る、CRLF、BOM、空ファイル |
| U4 | `nm_ipbin` | `.gdi`：通常、引用符付きの名前、BOM、トラック 2 本だけ、壊れた行。IP.BIN：2048 と 2352、マジック違い、タイトルの空白の整理 |
| U5 | `nm_text` | ASCII 以外 → `?`、幅で切る |
| U6 | `nm_list` | 並び（番号 → 名前）、重複番号、上限超え |
| U7 | `nm_input` | 立ち上がり、300 ms／80 ms のリピート、同時押し |
| U8 | `ngtp_frame` | 往復、CRC 不一致、長さ超過、マジック違い、途中までのヘッダ |

**一覧と起動（`ui`）**

| # | シナリオ | 期待 |
|---|----------|------|
| L1 | `02`（GDI + `GAME.CFG`）、`03`（CDI + `name.txt`）、`04`（GDI、IP.BIN のみ）、`05`（`.NGP` だけ）、`06`（空）、`MYGAME`（名前フォルダ）、`01`（メニュー） | 画面の行が期待の文字列と一致。`01` は出ない。`05` は incomplete、`06` は empty |
| L2 | 走査中に書き込みが 1 回も無い | `sim_cmd_writes == 0`、`sim_begin_calls == 0` |
| L3 | A で `03` を起動 | パケットが `52 82 03 00`、その後 `00`（TEST UNIT READY）が READY 2 回まで、`HANDOFF bios` |
| L4 | 名前フォルダを起動 | `E0 05 ...`（9 バイトずつ）→ `E0 02 00` |
| L5 | 番号はあるがイメージが無い（模型がセンスキー 5） | ダイアログ「No image」、一覧に戻る、`HANDOFF` なし |
| L6 | NOT READY が続く | 上限（10 秒 + INI の時間）でダイアログ。A でもう一度待つ |
| L7 | UNIT ATTENTION が先に立っている（`--ua`） | 再送されて成功 |
| L8 | ファームが答えない（模型の機能を切る） | `NOFW`。セカンダリの読み出し 0 回 |
| L9 | FAT32 でないカード、MBR なし | `NOSD` と理由 |
| L10 | 読み取りのビット化け（`--rflip`） | 再読して同じ一覧。化け続ける（`--rflip-all`）→ 「SD read error」で止まり、固まらない |
| L11 | 300 フォルダ | ページ送り、カーソルの端の動き、走査中の操作 |
| L12 | `GDEMU.INI` `reset_goto=1`／なし | 情報画面と状態行の表示 |
| L13 | 情報画面で渡し方を切り替えて起動 | `HANDOFF reboot` |
| L14 | START → 確認 → A | `HANDOFF bios`、`52` を 1 回も送っていない |

**取り込み（`serve` + `netgd-send push`／試験用の生フレーム）**

| # | シナリオ | 期待 |
|---|----------|------|
| N1 | GDI を `push --slot auto` | 終了コード 0。フォルダ `02` に全ファイル、`name.txt` あり、`.NGP`・`NETGD.CKP` なし、`fatcheck.py` で破損なし、パーティション外への書き込みなし |
| N2 | CDI を `push --name X --mount` | `sim_end_mode == 2`、名前が `X` |
| N3 | 合言葉違い | `ERROR EAUTH`、`sim_begin_calls == 0` |
| N4 | `HELLO` なしで `PUT`、`WANT` 無しの `DATA`、オフセット違いの `DATA` | `ERROR ESTATE`、切断、SD は変わらない |
| N5 | CRC の壊れたフレーム | `ERROR EBADCRC`、切断 |
| N6 | 転送の途中で PC が切断 | サーバは待機に戻る。もう一度 `push`（resume）で完走、内容一致 |
| N7 | 転送の途中で電源断（`--cut` を 20 か所ほど。全位置の総当たりは netgd-recv 側が持っている） | どの位置でも `fatcheck.py` が通り、見えるイメージは完全なものだけ。再実行で完走 |
| N8 | 保護フォルダ（`01`）へ `--force` 付き `push`：確認で A／B | A：成功。B：`EPROTECTED` |
| N9 | 空き不足 | `DONE` 失敗 `ENOSPC`、SD は 1 バイトも変わらない |
| N10 | B で安全に止める（`--stopafter`） | `NGD:STOP`、終了コード 2、再開で完走 |
| N11 | `FINISH` なしで 5 分（`--idle-ms` で短縮） | セッションが閉じる（`sim_end_calls == 1`、指定 1） |
| N12 | `FINISH` の後 | 軽いマウントに戻り、一覧に新しいフォルダが出る（`ui` と `serve` をつないだ 1 本の試験） |
| N13 | `remote list`／`statfs`／`delete`／`verify` | netgd-recv の同じ操作と同じ `NGD:` 行 |
| N14 | セッション中に起動を試みる経路が無い | `sim_session` が 1 の間に `52 82` が 1 回も出ない（全シナリオの終わりで検査） |

全シナリオ共通の検査：`sim_unsessioned_writes == 0`、`sim_violations == 0`、終了時に `sim_session == 0`。

既存の試験：`tools/netgd-recv/test/run_tests.py` と `tools/netgd-send` の 42 件が変更後も全部通ること。

---

## 12. エラーの扱い

| 起きること | 表示 | 動作 |
|-----------|------|------|
| ファームが `OGDM` を返さない | `NetGD firmware not found` | `NOFW`。何も送らない |
| セカンダリが無い／FAT32 でない／セクタ 0 が読めない | `SD card: <理由>` | `NOSD` |
| 走査中の読み取りエラー | 状態行 | そのフォルダを飛ばす。3 連続で走査を止める |
| `52 82`・`E0 02` がセンスキー 5 | `No image in folder NN` | 一覧に戻る |
| READY にならない | `Drive not ready` | A：待ち直す／B：一覧 |
| BBA なし／アドレスなし | `BBA not found`／`No IP address` | 一覧に戻る |
| セッションを開けない（古いファーム、GPT のカード） | `NGD:ERROR ... EFW` を表示 | `DONE` 失敗。取り込み画面に残る |
| `put` の失敗（`ENOSPC`・`EFRAG`・`ECRC`・`EBUS`・`EIO`・`EPROTECTED`） | `NGD:ERROR` 行 | `DONE` 失敗。セッションは開いたまま、次の要求を待つ |
| LAN 断・PC の無応答 | `connection lost` | 接続を閉じて待機。5 分でセッションを閉じる |
| プロトコル違反 | `protocol error` | `ERROR` を返して切断 |
| セッション終了（END）が失敗 | `Could not close the session: reset the console` | 一覧に戻らず `NOSD` 相当の終端（リセットでファームが戻す。F5） |
| G1 が 60 秒固まる | — | `dev_kos.c` のウォッチドッグが `arch_abort()`（既存） |

安全の決まり（まとめ）：

1. 書き込みセッションの外では、セカンダリに書き込みコマンドを 1 回も出さない（`ngblk_readonly`。ファームもメディアがある間は ABRT。二重の守り）。
2. セッションを開くのは、利用者が取り込み画面に入り、合言葉の合った PC が操作を送ったときだけ。
3. どの出口（起動、BIOS へ、取り込み画面を出る、エラー）でも、先にセッションを閉じる。
4. 保護フォルダ（`GDEMU.CFG` の既定、`01`）の上書き・削除は、PC の force 指定と本体での確認の両方があるときだけ。
5. MBR とパーティションの手前はファームが拒否する（F7）。アプリはフォーマットしない。
6. ファーム更新（`0xF9`）は v1 のメニューからは送らない。

---

## 13. 実装の手順

各手順は 1 つのコミット（または 1 つの PR）。受け入れ条件はすべてこの PC で確かめられるもの（KOS のコンパイルは CI）。

| # | 内容 | 受け入れ条件 |
|---|------|-------------|
| 1 | netgd-recv への追加（§4.3）：`ngblk_readonly`、`ngfs_mount_light`、`fwctl` の 6 関数、`fwctl_sim` の `0x52`・`0x00` の模型。`test_host` に `light <img> <sectors>`（軽いマウントで `ngfs_list` を呼ぶ）と `gdemu <n>` を足して試験 | netgd-recv の既存の試験が全部通る。新しい試験：軽いマウントで読み出しコマンドが FAT 走査より 2 桁少ない、軽いマウント中の `ngblk_write` が `-1` で `sim_cmd_writes` が増えない、`52 82 lo hi`・`52 81 55/44`・`52 00`・`E0 05/02`・`00` のパケットのバイト列が §5.2 と一致、UNIT ATTENTION の再送 |
| 2 | `tools/netgd-menu` の骨組み：`nm.h`、`nm_text`、`nm_slot`、`nm_cfgparse`、`nm_ipbin`、`nm_list`、`nm_input`、`Makefile.host`、`menu_host selftest`、`run_tests.py`、`build.yaml` に 1 行 | U1〜U7 が通る。gcc の警告 0（`-Wall`）。Windows（MSYS2 gcc）と Linux の両方でビルドできる |
| 3 | `nm_screen`、`nm_scan`、`nm_ui` の `BOOT`・`NOFW`・`NOSD`・`LIST`・`INFO`、`plat_host.c`、`menu_host ui` | L1・L2・L8〜L12 が通る |
| 4 | `nm_launch`、`DIALOG`、`HANDOFF`、START の確認 | L3〜L7・L13・L14 が通る |
| 5 | KOS 層：`main_kos.c`、`plat_kos.c`、`Makefile`（§9.2）、`build.ps1`、`ip.txt`、`make cdi`、`README_ja.md`（初回確認リスト付き）。取り込みは「not built」と出す仮の `net_kos.c` | `kos.yaml` が成功し、成果物に `netgd-menu.elf`。netgd-recv・netgd-fwupd・gdprobe の ELF も今までどおり出る。**ここが実機の 1 回目（一覧と起動）** |
| 6 | `ngtp_frame`、`ngtp_srv`、`nm_ui` の `NET_START`〜`RECV_CLOSE`・`CONFIRM`、`net_host.c`、`menu_host serve`。試験は Python の生フレームのクライアント（試験フォルダ内） | U8、N3〜N5、N9、N11、N13（生フレームで）が通る |
| 7 | `netgd-send`：`ngtp.py`、`push`、`remote`、`prepare` の切り出し、`name.txt` | netgd-send の既存 42 件が通る。N1・N2・N6〜N8・N10・N12・N14 が通る。`tools/netgd-send/README_ja.md` に `push` の節 |
| 8 | `net_kos.c`（§10 K2〜K4） | `kos.yaml` が成功。**実機の 2 回目（取り込み）** |
| 9（任意） | UDP の探索（`netgd-send discover`、`push` の `--host` 省略） | ホスト試験：127.0.0.1 で探索 → 応答 |
| 10（任意・実機の後） | ファーム更新の統合：`../netgd-fwupd` の `fwcore.c`・`fwmsg.c`・`g1dev.c` を参照。NGTP に `FWPUSH`（0x0A）を足し、RAM に受けて `fw_parse_file` → 確認画面（netgd-fwupd と同じ 4 ボタン 3 秒）→ `fw_update`。セッションを閉じてから。対策 B が実機で動いた後に着手 | netgd-fwupd の偽デバイスでのホスト試験、`0x59` を送らないことの検査 |
| 11（任意） | 箱絵：フォルダの `NETGD.ART`（16 バイトのヘッダ `"NGAR"`・版・幅・高さ + RGB565 LE、128×128 固定）。カーソルが 300 ms 止まったら読み、画面の右に出す。PC 側は `netgd-send push --art <png>`（Pillow は任意の依存） | ホスト試験：ヘッダの検査、大きさ違いを無視、読み取りエラーで一覧が止まらない |
| 12（任意・実機の後） | 実機の数字で決めるもの：走査のキャッシュ、直接起動のローダー、CDI のタイトル、T2 の 2 スレッド化、日本語の文言 | 別の設計を書いてから |

手順 1〜4 は KOS が要らない。手順 5 と 6〜7 は並行できる（ファイルが重ならない）。

---

## 14. 実機でしか分からないこと

| # | 問い | 影響 |
|---|------|------|
| H1 | `INIT_CDROM` を外した KOS アプリを SD のイメージから起動できるか。起動後に `fw_xfer`（プライマリへの生 ATAPI）と `g1ata`（セカンダリ）が両方動くか | 動かなければ `INIT_CDROM` を戻し、KOS の cdrom ドライバと G1 の使い方を調整する |
| H2 | `52 82` の後で `arch_menu()` に戻ると、BIOS が新しいディスクを認識して起動できるか | 既定の渡し方（§5.4） |
| H3 | `arch_reboot()` は G1 のハードリセットを出すか（`reset_goto=1` でメニューに戻ってしまうか）。ファームのログ「hard reset」で分かる | 再起動方式を既定にできるか。`reset_goto` の推奨値 |
| H4 | イメージをマウントしたまま、セカンダリの読み出しが正しく通るか（プライマリの状態を乱さないか） | D2。だめなら一覧の読み取りも排出してから行う |
| H5 | 走査の時間（1 コマンドあたりの ms、100 本での合計） | キャッシュの要否（手順 12） |
| H6 | KOS の TCP + BBA の受信速度、`WANT`／`DATA` の往復での実効速度 | D6 の見直し（2 スレッド、先読み） |
| H7 | BBA なしで `net_init` がすぐ戻るか。DHCP の待ち時間。固定 IP（本体のフラッシュの設定）が使われるか | `NET_START` の表示と時間切れ |
| H8 | 同じ番号のフォルダが 2 つあるとき、ファームはどちらをマウントするか | §6.1 の重複の扱い |
| H9 | セッションを閉じた後（END 指定 1）、`52 82` がすぐ通るか。UNIT ATTENTION の出方 | `RECV_CLOSE` → 起動の流れ |
| H10 | PAL（50 Hz）・VGA での表示、`bfont` の 53 桁が画面に収まるか（オーバースキャン） | 余白の調整 |
| H11 | netgd-recv の初回確認リスト（README の 1〜12）のうち、DMA・FLUSH・FNV 照合・マウント時間 | メニューの取り込みも同じ経路を使う |

実機で確かめるまで、README と PLAN には「実装済み・実機未検証」と書く。

---

## 15. 関連

- G2 の設計：`docs/design/bba-sd-write.md`（5.1 マニフェスト、5.2 確定、5.5 安全の決まりはそのまま有効。5.4 はこの文書の §8 で置き換え）
- 受信コア：`tools/netgd-recv/`（R48、R62）。PC 側：`tools/netgd-send/`（R30）。ファーム更新：`tools/netgd-fwupd/`（R52）
- `0x52`：`mcu/netgd/src/ide.rs` `handle_gdemu_cmd`（R13、R14）、`docs/research/web-findings-2026-11.md` §1
- 番号フォルダ：`common/discimg/src/slot.rs`。`GDEMU.INI`・`GAME.CFG`：`common/gamecfg/`
- CI：`.github/workflows/kos.yaml`、`tools/ci/kos-build.sh`（R35、R87）
