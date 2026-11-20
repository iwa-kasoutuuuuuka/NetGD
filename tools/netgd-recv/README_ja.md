# netgd-recv（G2 の Dreamcast 側アプリ、T1 のみ）

BBA（ブロードバンドアダプタ）経由で、PC から送られたゲームイメージ（`.gdi` / `.cdi`）を **NetGD の SD カードに書き込む** KallistiOS アプリ。設計は `docs/design/bba-sd-write.md`（3.2 案 A、4.2、4.5、5 章、6 章、8 章）。PC 側の半分は `tools/netgd-send`。

> **重要：実機では一度も動かしていない。**
> ホスト（PC の gcc）での試験だけが済んでいる（下の「テスト」）。KOS 用の部分（`src/main.c` `src/plat_kos.c` `src/dev_kos.c` `src/fwctl.c`）は **この PC にツールチェーンが無いのでコンパイルもしていない**。最初の実機検証では、下の「KOS 専用部分の初回確認リスト」を上から順に確かめること。
> SD カードを書くアプリなので、**最初は捨てても困らない SD カードで**試すこと。

## 何をするか

`dc_auto` が `netgd-recv.elf` を dcload-ip で送って起動すると、アプリは PC の `/pc/netgd/<ユニット>/job.ngj`（`netgd-send prepare` が作る）を読み、書かれたとおりに SD カードを更新する。

1. ファームの `0xE0` VERSION で NetGD のファームか確認 → **書き込みセッション開始**（GD-ROM イメージの排出、ボタン無効、MBR 保護）。
2. セカンダリ IDE ディスク（= SD カード）の **第 1 パーティションの FAT32** を FatFs（ChaN R0.15b）+ KOS `g1ata` でマウント（FAT32 でなければ何も書かず `EFS`）。
3. `job.ngj` の操作を上から順に実行：`statfs` / `list` / `put NNNN [new|resume|overwrite]` / `delete` / `verify`、最後に `finish`。
4. `put`：`NNNN/manifest.ngm` を読み、空き容量を確認 → フォルダ作成 → `NETGD.MF`・`NETGD.CKP` → 全ファイルを `f_expand` で**連続領域に先に確保**（記述子は `DISC.NGP`）→ `/pc/` から 64 KiB ずつ読んで書く（8 MiB ごとにチェックポイント）→ ファイルごとの CRC32 照合 → 読み戻し検証 → **確定**（`NETGD.CKP` を「完了」にし、ディレクトリエントリの拡張子 3 バイトだけ `NGP`→`GDI`/`CDI` に書き換え。SD の 1 セクタの書き込み 1 回）→ `NETGD.CKP` を削除。
5. `finish`：FatFs をアンマウントし、セッションを終了（`mount <フォルダ>` ならそのフォルダをマウント、`default` なら `GDEMU.CFG` の既定）。
6. 結果を `result.ngr`（4 KiB 以下の行ごとに `fwrite` + `fflush`）とコンソール（`NGD:` 行）に出して `exit(0)` で dcload に戻る。

途中でどこで電源が落ちても、`.NGP` の間はメニューにもファームにもイメージとして見えない。もう一度同じ操作（`resume`）を実行すると、最後のチェックポイントから続く。

## 使い方（dc_auto + netgd-send）

`tools/netgd-send/README_ja.md` の「T1 の流れ」と同じ。このアプリ側で必要なこと：

1. ビルド（DreamSDK のシェル、または `.\build.ps1 -BbaTools <Dreamcast-Dev-Tools-for-BBA のフォルダ>`）で `tools/netgd-recv/netgd-recv.elf` を作る。
   - **`tools/netgd-send/integrations/dcdevtools/netgd.ps1` の `Elf` は今 `dc\netgd-recv\netgd-recv.elf` を指している**（まだ存在しない位置）。このアプリは `tools/netgd-recv/` にあるので、プリセットの `Elf` をそこへ向けるか、ELF をコピーすること（netgd-send 側はこの作業では触っていない）。
2. PC：`python tools\netgd-send\netgd-send.py prepare <image.gdi> --job <ドライブ>:\netgd\dc1 --name MYGAME`（`--mode resume` が既定のプリセットなら再実行がそのまま再開になる）。
3. `.\scripts\dc_auto.ps1 -Preset netgd` で送って実行。画面とコンソールに進捗（`NGD:PUT ...`）が出る。
4. 終わったら PC で `netgd-send.py collect --job <ジョブフォルダ>`（終了コード 0 = 成功、2 = 途中・再開可、1 = 失敗）。

ジョブフォルダの場所：既定は `/pc/netgd/dc1`（`-DNG_JOBDIR_DEFAULT`、`build.ps1 -JobDir` で変更）。そこに `job.ngj` が無ければ `/pc/netgd/` の下で `job.ngj` を持つフォルダがちょうど 1 つのときそれを使う（複数あれば選ばずに止まる）。**`dc-tool-ip` はジョブフォルダと同じドライブで起動すること**（`/pc/` の規則）。

追加の合図ファイル（ジョブフォルダに PC が作る）：

| ファイル | 意味 |
|----------|------|
| `reload.req` / `cancel.req` | 1 秒に 1 回見て、あれば現在のチャンクを書き終えて安全に止まり `exit(0)`（チェックポイントは直前の 8 MiB 境界。`NGD:STOP`、`NGD:END ok=0`、`collect` は 2）。コントローラの **B ボタン**でも同じ。 |
| `force.req` | `GDEMU.CFG` が指す既定のフォルダ・`01` を **上書き・削除してよい**（設計 5.5 の「明示の強制指定」。netgd-send に対応する指定が無いので、ファイルにした） |
| `fragok.req` | 連続領域が取れないとき、断片化した確保で続行してよい（既定は `EFRAG`。読み出しが遅い経路になりうる） |

## 安全（設計 5.5、8）

- 書くのは**第 1 パーティションの FAT32 だけ**。フォーマット機能なし。パーティション表（MBR）には書かない（ファーム側でも書き込みを拒否する）。
- 空きが「必要量 + 16 MiB」に満たなければ、**何も書く前に** `ENOSPC need=… free=…`（フォルダも作らない。`overwrite` でも古いイメージを消す前に判定）。確保が先に全部終わるので、転送の途中で満杯にならない。
- `GDEMU.CFG` の `image=` が指すフォルダと `01` は、`force.req` が無い限り上書き・削除しない（新規作成は可）。`delete` は `NETGD.MF` か `.gdi`/`.cdi`/`.NGP` があるフォルダだけ、ルートは不可、サブフォルダがあれば失敗。
- 書いたセクタごとに（FAT・ディレクトリも）ファームの STATUS で FNV-1a と照合し、違えば同じ LBA に書き直す（3 回まで。だめなら `EBUS`）。ファームが検査値を持たないとき（VERSION の機能 bit1 = 0）は読み戻し検証を**必須**にする。読み戻しは既定で有効（`ng_cfg.readback`）。
- どの終了経路でもセッションを閉じる（`WSESSION_END`）。マウント指定が失敗したときは既定のイメージを戻す（`IMAGE_CTL MOUNT 0xFF`）。
- dcload-ip は認証も暗号化も無い。信頼できる LAN だけで使う。`/pc/` へ書くのは結果ファイルだけ。
- **dc-tool-ip を強制終了しない**（noexec になり電源の入れ直しが必要）。中断は `cancel.req` か B ボタン。

## 結果ファイル `result.ngr`

`tools/netgd-send/README_ja.md` の「結果ファイル」と同じ行（`HELLO` `PUT` `RESUME` `FILE` `DONE` `ERROR` `FINISH` `END`）に加えて、`netgd-send collect` が無視する行を出す：`NGD:STATFS free=… total=… cluster=…`、`NGD:LIST <complete|partial|other> <パス>`、`NGD:VERIFY "<フォルダ>" ok|fail …`、`NGD:DELETE "<フォルダ>" ok`、`NGD:STOP <番号>`。`NGD:ERROR` の `msg="…"` は人間向けの説明。

エラーコード：設計 5.4 の一覧に加えて、**`EFS`**（第 1 パーティションが FAT32 でない。設計に名前が無いので追加）、`EJOB`（job.ngj が読めない）、`ESRC`（`/pc/` の元ファイルが読めない）。

## 構成

```
Makefile / build.ps1     KOS 用（未ビルド）           Makefile.host / build.sh   ホスト試験用
src/main.c               起動、画面（bfont）、結果表示           [KOS 専用]
src/plat_kos.c           時計、/pc/ パス、コンソール、B ボタン、get_fattime   [KOS 専用]
src/dev_kos.c            ngdev を KOS g1ata で（DMA/PIO、32 バイト整列、60 秒ウォッチドッグ）   [KOS 専用]
src/fwctl.c              ファーム側コマンドの層（0xE0 VERSION/BEGIN/END/STATUS/MOUNT_NAME、ATAPI を G1 レジスタで直接）  [KOS 専用]
src/ngblk.c              書き込みごとの FNV-1a 照合と書き直し
src/diskio.c             FatFs の diskio
src/ngfs.c               FAT 操作（確保、チェックポイント、確定 ngfs_commit_ext、削除、一覧、probe）
src/recv_core.c          put / verify / delete / list / finish の状態機械（転送路に依存しない）
src/xport_pc.c           T1：job.ngj、/pc/ からの読み出し、NGD: 行、result.ngr、reload/cancel
src/manifest.c crc32.c   マニフェスト、CRC32/FNV-1a
third_party/fatfs/       FatFs R0.15b（ChaN、ライセンス表記つき。ffconf.h だけ変更）
test/                    ホスト試験（dev_file.c = ファイルを SD に見立てる、fwctl_sim.c = ファームの模型、run_tests.py、fatcheck.py）
```

ファームのパケットの形はすべて `src/fwctl.c` の 1 ファイルにある（ファーム側の実装 `mcu/opengdemu/src/ide.rs` と `common/discimg/src/wsession.rs` に合わせた）。

## テスト（ハードウェア無し）

```
python tools\netgd-recv\test\run_tests.py            # 全部（数分。gcc が要る）
python tools\netgd-recv\test\run_tests.py TestBasics # 動作確認だけ
```

`gcc` でアプリの中核（マニフェスト、チェックポイントと再開、確定、`result.ngr`、FatFs 経由の FAT 層）をビルドし、FatFs の `f_mkfs` で MBR 付き FAT32 イメージを作って、`netgd-send prepare` で作った本物のジョブフォルダ（`/pc/` はホストのフォルダに置き換え）を実行する。確認する内容：全ファイルの内容とパーティション外への書き込み無し、連続確保、`.NGP` と `NETGD.CKP` が残らない、`netgd-send collect` が終了コード 0 / 2 を返し再開位置の CRC も合う、FatFs で読み戻した CRC、既存フォルダの扱い（new / resume / overwrite / 保護）、ENOSPC で SD が 1 バイトも変わらない、断片化（EFRAG と `fragok.req`）、G1 のビット化けの模擬（書き直し）、途中停止と再開、**電源断の総当たり**（書き込みセクタ数 N = 0 から最後まで。どの N でも独立した FAT リーダ `fatcheck.py` で構造の破損が無く、見えているイメージは古いか新しい完全なものだけ、再開すると完走して取りこぼしクラスタは確保中の分だけ）。

## 未実装・今後

- T2（NGTP、単体 TCP）、`dcload_type != IP` のときの起動は未実装（その旨を表示して終了）。
- `fsck.fat`（Windows には無い）の代わりに、`test/fatcheck.py`（FAT32 の独立した読み取り実装）で検査している。
- 実機でしか確かめられないこと：下のリスト。

## KOS 専用部分の初回確認リスト（実機・KOS ビルドで最初に見る所）

1. **コンパイル**：`main.c`（`bfont_draw_str` の引数と `vram_s`、`KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` の書き方、`dcload_type`）、`plat_kos.c`（`cont_state_t`、`maple_dev_status`）、`dev_kos.c`（`memalign`、`thd_create`、`arch_abort`、`timer_ms_gettime64`）、`fwctl.c`（`g1_ata_mutex_lock/unlock` と `g1_ata_select_device` が KOS 2.3.0 のヘッダで公開されているか。設計 2.3 で「要確認」とした点）。KOS 2.3.0 の `g1ata.h` にあることは確認したが、ビルドはしていない。
2. **`INIT_CDROM` を外してよいか**：外す理由は、セッション中に KOS が GD-ROM に触らないようにするため。外すと起動時の GD-ROM 初期化が無くなる。dcload-ip からの起動で問題が出たら外す。
3. **ATAPI の直接発行**（`fwctl.c` の `atapi()`）：プライマリ（GD-ROM）への PACKET コマンドを G1 の ATA レジスタで直接出している。BSY/DRQ の待ち方、バイトカウント、データ読み出し。VERSION が `OGDM` を返すかが最初の確認。KOS の GD-ROM ドライバや BIOS が同じバスを同時に使わないこと。
4. **セカンダリの検出と容量**：`g1_ata_init` が成功し、`g1_ata_blockdev_for_device(0, …)->count_blocks` が SD の全セクタ数を返すか（`dev_data` を `free` している。KOS 2.3.0 で形が同じか）。
5. **DMA**：ファームの VERSION 機能 bit2 が立つとき `g1_ata_*_lba_dma` を使う。書き込みで -1 が返るなら PIO（`ngdev_use_dma = 0`）に落とす。バッファは 32 バイト境界（アプリは `iobuf` と `bounce` を整列済み）。
6. **`g1_ata_flush`（FLUSH CACHE）が成功するか**（ファーム F2）。失敗すると FatFs の `CTRL_SYNC` が `FR_DISK_ERR` になる。
7. **STATUS の FNV 照合が通るか**：`NGD:HELLO caps=7` のとき `ngblk_stats.fnv_mismatches` が 0 のままか。最初の実行で `fnvbad` が出るなら、ファームの FNV の取り方（コマンド全体で 1 つの値）とアプリの照合（`ngblk_write`）を突き合わせる。
8. **`/pc/` の読み出し**：64 KiB 単位の `fread`、再開位置の `fseek`（2 GiB を超える位置は読み捨て）。読み出し速度を測る（設計 G2-3b）。
9. **セッション終了後に BIOS がディスクを認識するか**（設計 R10）：`WSESSION_END` の後 `exit(0)` で dcload に戻り、さらに本体メニューへ戻れるか。
10. **`localtime`/`time`**（`get_fattime`）が RTC を返すか（返さなくても動くが、ファイルの日付が 2025-06-01 になる）。
11. **スタック**：`ng_probe_t`、`FILINFO`（LFN 255 文字）、`list_rec` の 4 KB 配列が KOS のメインスレッドで足りるか（足りなければ `static` にする）。
12. **BBA の速度と dcload の呼び出し**：1 秒に 1 回の `reload.req`/`cancel.req` の確認が転送を邪魔しないか。
