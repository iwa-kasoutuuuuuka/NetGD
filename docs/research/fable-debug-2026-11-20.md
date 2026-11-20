# Fable による全体のデバッグ（2026-11-20、LOG R89）

ユーザーの指示「fable で全体をデバッグ」で、Fable を3並列（読むだけ、リポジトリは変更なし）で走らせた結果。
対象は HEAD 84a4d180。**実機での確認はしていない。** 「確認済み」は、ビルド・ホストテスト・使い捨てのテスト、またはコードを読んで疑いの余地がないものを指す。
Opus は各領域の主な指摘をコードで確かめた（下の「Opus 確認」）。

担当：
- A：ファームの読み込み経路（`mcu/netgd/src`、`embassy-atsam3`、`sdfast`、`fatmap` の使われ方）
- B：自己更新と起動の流れ（`mcu/bootcore`、`resident.rs`、`fwupdater`、`tools/fwimage`）
- C：G2 の書き込み経路（`tools/netgd-recv`、`tools/netgd-send`、`common/discimg` の解析、`common/fatmap`）

## A. ファームの読み込み経路

### 確認済み
| # | 重さ | 場所 | 内容と起きること | 直し方の案 |
|---|---|---|---|---|
| A1 | 高〜中 | `ide.rs:3299` `wait_host`、`push_buf_to_fifo`・`stream_from_sd`・`stream_pio_from_sd`・`handle_disk_read` の DMA ループ | 同じデバイスへの新しいコマンド（DEVICE RESET 0x08、新しい PACKET）で、複数チャンクの転送が止まらない。FPGA はコマンドの書き込みで FIFO を空にするが `dma_mode`・`pio_rd_fc` は立ったまま。PIO では次のブロックの BSY・バイト数・DRQ+INTRQ を新しいコマンドの CDB の段階に重ねて書き 10 秒待つ。DMA では古いセクタを詰め、5.2 秒後に新しいコマンドのバイト数・IRR を上書き | `wait_host` に自デバイスの `command_pending()` で抜ける出口を足し、状態・バイト数・IRR を書く前に毎回確かめる |
| A2 | 中 | `ide.rs:2547`→`:2677`（**Opus 確認**） | DMA の CD_READ が途中で SD 読み取りに失敗しても `complete(0, STATUS_IDLE)`（成功）を返す。PIO は ERR_ABRT（`:2809`） | `sectors_done < count` ならエラーの状態と sense |
| A3 | 中 | `fat.rs:272-285` `resolve_long` | 8.3 の別名に 0x80 以上のバイトがある長い名前のトラック（日本語・アクセント付き）をマウントできない。`heapless::String<12>` に UTF-8 で入れて溢れる、`create_from_str` が小文字を大文字にし `[ \ |`（DBCS の2バイト目）を拒否。使い捨てテストで cp932 の名前4例すべて失敗 | 生の 11 バイトか `fat::Entry`（先頭クラスタとサイズ）をそのまま使う |
| A4 | 低〜中 | `ide.rs:986`（**Opus 確認**）、`main.rs:219` | `MEDIA_PRESENT` の初期値が true で、起動時のマウント失敗（空のカード、01 が無い等）でも false にならない。TEST UNIT READY は準備完了、TOC は ABRT、ディスクへの書き込みは「先に取り出せ」で拒否 | `ide_task` を始める前に `gdi_ref().is_some()` を入れる |
| A5 | 低（ただし毎回の起動で止まりうる） | `gdi.rs:890`（**Opus 確認**）、`ide.rs:1961` | GDI のセクタサイズを検査していない。0 だと `entry.size / sector_size` で 0 除算のパニック。既定のイメージなら毎回の起動でパニック。2352 より大きいと遅い経路でスライスの範囲外 | `PoolSink::push` で 2048/2336/2352 以外を拒否 |
| A6 | 低 | `sdcard.rs`、`main.rs:269` | feature `sd-bench`（`Instant` の import 漏れ）と `fpga-diag`（`_free_run` と `free_run`）がコンパイルできない。実機の最初の診断で使うもの | import と名前を直す |
| A7 | 低 | `ide.rs:861`、`:790` | PACKET（0xA0）と DEVICE RESET（0x08）がデバイスの役割を見ていない。副（ディスク）で 0xA0 は 10 秒 BSY の後 ABRT、0x08 はディスクのレジスタに ATAPI の署名を入れ CDDA を止める | `role == Role::Packet` でなければ ABRT |
| A8 | 低 | 下の各所 | `gdi.rs:44` `MAX_GDI_BYTES = 4096` は名前 128 B（R86）だと約 45 トラックで切れる／`ide.rs:3539` 書き込み FIFO の量が足りなくても CMD25 に流して成功扱い／`ide.rs:2997` 割り当て長 0 で空の FIFO に DRQ と INTRQ 2回／`ide.rs:943` `size_disk_from_mbr` に `looks_like_bpb` の確認が無い／`fat.rs:211` `lookup` がディレクトリの循環で無限ループ（ウォッチドッグ無効）／SUB_MOUNT の NO_SELECTION と `52 82 00 00` は何もマウントしなくても OK／MOUNT_NAME と `GDEMU.CFG image=` は非 ASCII のフォルダ名に一致しない | — |

### 実機が要る疑い
- A-a（中）FIFO の量の非同期の1回読み：`ide.rs:2836` `room()` で 0x800→0x7FF の繰り上がりの途中を読むと、空き 4092 と誤って DMAC が溢れる（FPGA の `w_en` は `w_rdy` を見ない）。`recv_packet`（2929）で 3→4 を 7 と読むと CDB の末尾が古い。2回同じ値になるまで読むか、FPGA で読み出しの瞬間に固定する。
- A-b（低〜中）転送中のホストのリセットの後に状態・INTRQ を書く経路が残る（`recv_packet` の失敗 1335、`handle_disk_read` の最後、`finish_pio_in` 3593、中断された `recv_data_out` の後の SET_MODE）。
- A-c（低）`ack_command` が pend を無条件に 0 にする（取り込みと ack の間のコマンドが消える）。`ide_task` が主・副を1回ずつしか見ない（エッジが来ない取りこぼし）。
- A-d（低）HSMCI：CMD12 を待つ間のデータエラーも判定に入るので、コントローラが切られたブロックにエラーを立てると CMD18 が毎回1回失敗し High-Speed を永久に落とす／短い CMD25 の後にリセットしない／上限の無い待ちループ／SDSC カードはブロック番地扱いで誤読。
- A-e（低〜中、性能）遅延エクステントの窓が全ファイルで1つ。CDI（1ファイル）で CDDA とデータを交互に読むと毎回 FAT を先頭からたどり直す（音声のバッファ約 30 ms を超えうる）。
- A-f ATA の PIO 読みの完了で INTRQ が1回多い／DMA のディスク読みが 0x50+INTRQ で始まる／2E の CDDA のリング（NFC SRAM 0x20100000）への DMAC の書き込みは未確認。

### 問題なし
DMA の整列（`CD_READ_BUF` は配置の偶然で整列。`#[repr(align(4))]` 推奨）、2E `resident,sd-fast` のスタック（使える 7,160 B、静的な最深 約 6.2 KB、残り約 1 KB で薄い）、
書き込みとキャッシュの無効化、CMD25 をまたぐ待ちの排他、LBA48、R75 の HSMCI のエラーの累積、`fatmap` の `Window` と LFN、整数の幅、共有の static、feature の組み合わせ（`sd-bench`・`fpga-diag` を除く）。

## B. 自己更新と起動の流れ

**基板を壊す種類のバグは今あるコードには無い。** bootcore 133 件、fwupdater 13 件、fwimage 48 件（2 skip）通過、`--features resident` のビルド成功。

### 確認済み
| # | 重さ | 場所 | 内容 | 直し方の案 |
|---|---|---|---|---|
| B1 | 形式2で高 | `tools/fwimage/fwimage.py:596-636`、`:214-224` | `fwimage build --target 4e` が 2E の ELF を通す（SP 0x20004000 は 4E でも有効、プレーン1が無いのは警告だけ）。今の 4E 基板（形式2）に書くと FPGA が起動せず SWD でしか戻せない | `build` で SP が対象の SRAM0 の終わりと一致することを要求、またはプレーン1の中身が無い 4E のイメージをエラーに |
| B2 | 中〜低 | `bootcore/src/update.rs:405`→`already_installed`→`:289`（**Opus 確認**：`set_fws` は `ewp_page` の中だけ） | `sd_recover_ex` が FWS を入れずにプレーン1を読む。取り決め K7 では常駐部が入れることになっている。FWS=0・96 MHz で読むとごみ→「未導入」と判断→毎回書き直し（摩耗と不要な電源断の窓、壊れはしない）。模型は FWS=4 から始まるので見えない | `sd_recover_ex` の最初で `set_fws(FWS_RUN)`、模型は「未設定」から始めて違反を数える |
| B3 | 低 | `update.rs:289`、`:416` | 印を入れたビットストリームの `payload_crc` は CRC の剰余で常に 0x2144DF1C。比較は「長さの合う自己整合のイメージ」しか示さない（アプリの末尾が `bs_stamp` を含むので今は安全） | プレーン1の最後の語もファイルと比べる |
| B4 | 低 | `flash.rs:27` | 存在しない `tests/watchdog.rs` を参照（実際は `tests/resident.rs:484`） | 参照を直す |

### 設計・実機の疑い（常駐部をロックする前に決める）
- B5（中）EWP のタイムアウトで RAM のルーチンからフラッシュのコードに戻ると、そのプレーンは読めないままでフォールトする。タイムアウトでは戻らない（ウォッチドッグに任せる、または RAM から `RSTC_CR`）と `Flash` トレイトに書く。
- B6（中〜低、ロック後は凍結）要求による復旧の途中でリセットされると、REQ のループ防止が消える（GPBR0 は復旧前に消し、REQ は引き渡しの時の GPBR1 にしか無い）。要求→ハング→16 秒→アプリ→要求…が続く。GPBR0 に「要求済み・進行中」の第2の値を入れる案。GPBR0 の意味が変わるので SLB の前に決める。
- B7（低、記憶による）WDT_CR の巻き戻し直後（約 92 µs 以内）の WDT_MR の書き込みを禁じる注記が後継の SAM にある。巻き戻しは `handoff()` の最初に。
- B8（低）アプリが無効で、`erase_head` の後に復旧が失敗するカードだと、カードの確認のたびに約 480 ページを書き直し続ける。回数の上限を。
- B9（低）`sizeprobe` の下書きの罠：`lock_bits` が FCMDE を見ない、`ewp`・`slb` の前に FSR を読まない。5,902 B にはクロック初期化・本物の HSMCI 初期化などが入っていない。
- B10（低、今は安全）`fwupdater` の `resident` は S5 の準備ができていない（`lock_mask` がビット0を含む、`program_page` にページ 32 以上の確認が無い、`reset()` が SYSRESETREQ）。
- B11 4E の形式2の2イメージの更新で、コードを確定してからビットストリームを送る。間で失敗すると新しいコード＋書きかけのビットストリーム。`FORMAT.md` の「まだ安全でないこと」に書かれていない。
- B12（低）GLB が失敗すると全ビット1と読み、LOCKED と報告し SLB を試さない／`check_locks` が復旧を永久に拒否。

### 問題なし
フラッシュの番地とレジスタ（記憶によるデータシートの値）、コマンドの許可リストとページ計算、Rust・Python・C の CRC の範囲、リンカの配置（4E の ELF で確認）、`sd_recover_ex` の順序、起動の取り決め、アプリ側、FAT32 リーダー、形式の取り違えの拒否、共有のベクタ。
未確認：`tools/netgd-fwupd/main.c`・`g1dev.c`（KOS 側）は grep だけ。

## C. G2 の書き込み経路

**受信の中核にデータ破損のバグは無い。** netgd-recv quick 41 件、netgd-send 43 件、discimg・fatmap・gamecfg 通過。GDI 30 万・CDI 20 万・LFN 30 万件のファジングでパニックなし。

### 確認済み
| # | 重さ | 場所 | 内容 | 直し方の案 |
|---|---|---|---|---|
| C1 | 中〜高 | `tools/netgd-send/netgd_send/job.py:233-242`（**Opus 確認**） | 送り終わった `put` が `job.ngj` に残り、次の送信で再実行される。前回 `overwrite` なら、別のゲームを送るときに前のイメージを消して送り直す（その間の中断で前のイメージが消える）。`new` なら EEXIST で全体が失敗扱い、`resume` なら毎回全体を読み直す | 前回の `result.ngr` で `DONE ok` の項目を外す、または前回 `END ok=1` なら新しい `job.ngj` |
| C2 | 低〜中 | `job.py:251-253` | `cancel.req` しか消さない。`force.req` が残ると以後ずっと `01`・既定のイメージの保護が無効、`reload.req` が残ると毎回すぐ止まる | `prepare` で消すか警告、`force.req` は使ったらアプリが消す |
| C3 | 中 | `common/discimg/src/cdi.rs:81-86`（**Opus 確認**）、`netgd_send/image.py:14,227-235` | ファームの CDI の解析は v3.5（末尾からのオフセット）だけ。netgd-send は v2・v3（先頭からのオフセット）も受け付けるので、送って照合も通った後でマウントできない。テストの CDI は版の語を書かないので見えない | 最後の 8 バイトを読んで版で分ける（v2 は形が違うので拒否）。それまでは `check_cdi` を 0x80000006 だけに |
| C4 | 中（破損なし） | `job.py:72-85`、`:45-58` | `--placement auto`・`direct` でドライブをまたぐと、`/pc/` のパスからドライブ名が落ちて別のドライブを指す（ESRC か ECRC） | ジョブと同じドライブでなければ `JobError`（`--placement copy` を勧める） |
| C5 | 低 | `image.py:119`、`:140,166` | 非 ASCII のトラック名の .gdi を扱えない（latin-1 の `splitlines()` が 0x85 で切る） | `\n` だけで分け、UTF-8→ANSI→latin-1 の順に試す |
| C6 | 低 | `gdi.rs:19-20,28,31`、`manifest.py:8`、`ngfs.c:811` | 先頭の空行・空白を Python は許しファームは拒否／`number as u8`・`sector_size as u16` の黙った切り捨て／4 GiB 近くのサイズの境界のずれ | — |
| C7 | 低 | `fatmap/src/extent.rs:54`、`mcu/netgd/src/fat.rs:155-171` | 壊れた FAT の値（ボリュームを超えるクラスタ）で `cluster_lba` が u32 で桁あふれ。ファームの `fat_next` は範囲を見ないので、リリースでは誤った LBA を読む | `fat_next` でクラスタ数と比べる |

### 推測（未実行）
C8 長さ 0 のファイルを指すチェックポイントで再開すると最初からやり直す／C9 割り当て中の電源断でクラスタが漏れる（既知）／C10 クレーム後の `f_expand` の失敗で領域を表に戻さない／C11 `ngfs_default_image` が GDEMU.CFG の先頭 511 バイトしか読まない（長いコメントの後の `image=` だと既定のフォルダが保護されない）／C12 127 文字を超えるカード上の名前は切れる（ENOENT、誤削除は無い）／C13 同梱の `f_expand`（R0.15b）は一周したとき連続数を戻さない（ヒントの経路では起きない）。

### 実機が要る疑い
C-A 256 セクタの PIO 書き込みを KOS が 255+1 に分けると FNV の確認が毎回失敗（今は 128 セクタ以下なので潜在）／C-B 断片化した大きな GDI の名前指定の再マウントが 30 秒の BSY 待ちを超えうる／C-C `fw_xfer` の 400 ns の待ち／C-D `ngdev_sectors() = count − 1` は KOS の版次第。

### 問題なし
クラスタと LBA の計算、FAT32 だけに限る判定、空き領域の走査、領域の表、`f_expand` のヒント、チェックポイントと再開、確定の順序、プロトコルの取り決め、C の解析器、`fatmap`。
テストの限界：`dev_file.c` はすべての書き込みを1つの ATA コマンドかつセクタ単位で不可分として模し、テストのカードは FatFs の `f_mkfs` 製。
