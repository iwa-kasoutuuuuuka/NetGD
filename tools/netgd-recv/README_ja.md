# netgd-recv（G2 の Dreamcast 側アプリ、T1 のみ）

BBA（ブロードバンドアダプタ）経由で、PC から送られたゲームイメージ（`.gdi` / `.cdi`）を **NetGD の SD カードに書き込む** KallistiOS アプリ。設計は `docs/design/bba-sd-write.md`（3.2 案 A、4.2、4.5、5 章、6 章、8 章）。PC 側の半分は `tools/netgd-send`。

> **重要：実機では一度も動かしていない。**
> ホスト（PC の gcc）での試験だけが済んでいる（下の「テスト」）。KOS 用の部分（`src/main.c` `src/plat_kos.c` `src/dev_kos.c` `src/fwxfer_kos.c`）は **この PC にツールチェーンが無く、コンパイルは CI（`.github/workflows/kos.yaml`、KOS 2.3.0）だけ。実機では一度も動かしていない**。最初の実機検証では、下の「KOS 専用部分の初回確認リスト」を上から順に確かめること。
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

ジョブフォルダの場所：既定は `/pc/netgd/dc1`（`make NG_JOBDIR=/pc/…` または `build.ps1 -JobDir` で変更。値は**引用符なし**の `-DNG_JOBDIR_RAW=…` で渡し、C 側（`src/xport_pc.h`）で文字列化する。gdprobe の `HOST_DIR_RAW` と同じ方式で、`-D` に `\"…\"` を入れると `kos-cc` を通る間に壊れるため。したがって空白・`//`・引用符を含むパスは使えない。CI が渡す `HOST_DIR_REL` は gdprobe 用で、netgd-recv は見ない）。そこに `job.ngj` が無ければ `/pc/netgd/` の下で `job.ngj` を持つフォルダがちょうど 1 つのときそれを使う（複数あれば選ばずに止まる）。**`dc-tool-ip` はジョブフォルダと同じドライブで起動すること**（`/pc/` の規則）。

追加の合図ファイル（ジョブフォルダに PC が作る）：

| ファイル | 意味 |
|----------|------|
| `reload.req` / `cancel.req` | 1 秒に 1 回見て、あれば現在のチャンクを書き終えて安全に止まり `exit(0)`（チェックポイントは直前の 8 MiB 境界。`NGD:STOP`、`NGD:END ok=0`、`collect` は 2）。コントローラの **B ボタン**でも同じ。**読み戻し（`verify` を含む）の最中も 64 KiB ごとに確認する**。読み戻し中に止めたときのチェックポイントは「全ファイル書き込み済み」のままなので、再実行は読み戻しからやり直して確定する。 |
| `force.req` | `GDEMU.CFG` が指す既定のフォルダ・`01` を **上書き・削除してよい**（設計 5.5 の「明示の強制指定」。netgd-send に対応する指定が無いので、ファイルにした） |
| `fragok.req` | 連続領域が取れないとき、断片化した確保で続行してよい（既定は `EFRAG`。読み出しが遅い経路になりうる） |

## 安全（設計 5.5、8）

- 書くのは**第 1 パーティションの FAT32 だけ**。フォーマット機能なし。パーティション表（MBR）には書かない（ファーム側でも書き込みを拒否する）。
- **空き容量は FAT の全走査で数える**（`FF_FS_NOFSINFO=1`：FSInfo の空きクラスタ数を信用しない）。確保中の電源断で漏れたクラスタがあっても、FSInfo の過大な値で「空きがある」と誤判定しない。**時間：マウントごとに 1 回、FAT 全体を読む**（1 セクタ読みは 2 回読むので実質 2 倍。目安：256 GB・クラスタ 32 KiB で FAT 約 32 MiB、PIO で数十秒。実機で測ること）。
- 空きが「必要量 + 16 MiB」に満たなければ、**何も書く前に** `ENOSPC need=… free=…`（フォルダも作らない。`overwrite` でも古いイメージを消す前に判定）。確保が先に全部終わるので、転送の途中で満杯にならない。
- `GDEMU.CFG` の `image=` が指すフォルダと `01` は、`force.req` が無い限り上書き・削除しない（新規作成は可）。`delete` は `NETGD.MF` か `.gdi`/`.cdi`/`.NGP` があるフォルダだけ、ルートは不可、サブフォルダがあれば失敗。
- 書いたセクタごとに（FAT・ディレクトリも）ファームの STATUS で FNV-1a と照合し、違えば同じ LBA に書き直す（3 回まで。だめなら `EBUS`）。**ファームの VERSION 機能が bit0（SESSION）と bit1（CHECKSUM）の両方を持たなければ、何も書かずに `EFW` で止まる**（KOS の g1ata は PIO の書き込みエラーを返さないので、STATUS の FNV 照合が「届いた」ことの唯一の証拠。検査値なしの書き込み経路は削除した）。読み戻しは既定で有効（`ng_cfg.readback`）。
- **読み込み側のビット化け**：1 セクタの読み（FAT・ディレクトリ・FSInfo、確定時に書き戻すディレクトリのセクタ）は 2 回読んで一致を確かめ、違えば最大 4 回やり直し、安定しなければ `EIO`。複数セクタの一括読み（データの読み戻し）は 1 回だけで、CRC32 で確かめる。
- どの終了経路でもセッションを閉じる（`WSESSION_END`）。BEGIN の後のエラー（FAT32 でない `EFS`、読み取り不能 `EIO` など）でも END（既定のイメージを戻す）を出してから終わる。マウント指定が失敗したときは既定のイメージを戻す（`IMAGE_CTL MOUNT 0xFF`）。**ファームのコマンドが UNIT ATTENTION（センスキー 6。ロード・マウント・END の後に出る）で失敗したら 1 回だけ再送する**（VERSION・BEGIN・END・STATUS・MOUNT_NAME・MOUNT）。ウォッチドッグ（60 秒）が発動して `arch_abort()` する場合だけは、G1 バスが固まっているのでセッションを閉じられない（次の実行かリセットで戻る）。
- **フォルダ名の検査**：`put` のイメージ名と同じ規則（ASCII、空の要素なし、先頭・末尾の `/` なし、要素の末尾に空白と `.` なし、`..` なし、96 文字以内 = ファームの MOUNT_NAME の容量）を `delete`・`verify`・`list`（ルートの `/` は可）・`finish mount` の引数にも適用する（違反は `EMANIFEST`、`finish mount` は既定に戻して `FINISH fail`）。保護フォルダの比較は正規化後（先頭・末尾の `/` と `//` を畳み、要素末尾の空白・`.` を除く。GDEMU.CFG の `image=` も同じ）で行う。
- **直接書きの前提検査**：連続領域への直接書きの前に、開始クラスタが 2 以上、範囲がボリューム内、FAT の連鎖が本当に連続して終端していることを 1 回なぞって確かめる。満たさなければ `NGD:NOTE <番号> file=N slow-path reason="…"` を出して FatFs 経由の遅い経路で書く。
- **DMA**：マウント時（DMA が一度も成功する前）に DMA が失敗したら PIO で 1 回やり直し、成功したらそれ以降 PIO にする（コンソールに記録）。
- dcload-ip は認証も暗号化も無い。信頼できる LAN だけで使う。`/pc/` へ書くのは結果ファイルだけ。
- **dc-tool-ip を強制終了しない**（noexec になり電源の入れ直しが必要）。中断は `cancel.req` か B ボタン。

## 結果ファイル `result.ngr`

`tools/netgd-send/README_ja.md` の「結果ファイル」と同じ行（`HELLO` `PUT` `RESUME` `FILE` `DONE` `ERROR` `FINISH` `END`）に加えて、`netgd-send collect` が無視する行を出す：`NGD:STATFS free=… total=… cluster=…`、`NGD:LIST <complete|partial|other> <パス>`（サブフォルダは 16 件ずつ読み直して**打ち切らない**）、`NGD:READBACK <番号> file=i/n bytes=…/…`（読み戻し中、1 秒に 1 行以下。`/pc/` に動きを出して dc_auto に生きていると伝える）、`NGD:NOTE …`、`NGD:VERIFY "<フォルダ>" ok|fail …`、`NGD:DELETE "<フォルダ>" ok`、`NGD:STOP <番号>`。`NGD:ERROR` の `msg="…"` は人間向けの説明。

エラーコード：設計 5.4 の一覧に加えて、**`EFS`**（第 1 パーティションが FAT32 でない。設計に名前が無いので追加）、`EJOB`（job.ngj が読めない）、`ESRC`（`/pc/` の元ファイルが読めない）。

## 構成

```
Makefile / build.ps1     KOS 用（手元に KOS は無い。CI の kos.yaml がコンパイルする）    Makefile.host / build.sh   ホスト試験用
src/main.c               起動、画面（bfont）、結果表示           [KOS 専用]
src/plat_kos.c           時計、/pc/ パス、コンソール、B ボタン、get_fattime   [KOS 専用]
src/dev_kos.c            ngdev を KOS g1ata で（DMA/PIO、32 バイト整列、60 秒ウォッチドッグ。`busy_since` は IRQ 禁止で読み書き。容量は `count_blocks - 1`：KOS は max_lba + 1 を返す）   [KOS 専用]
src/fwctl.c              ファーム側コマンドの層（0xE0 VERSION/BEGIN/END/STATUS/MOUNT_NAME、UNIT ATTENTION の再送）。下の `fw_xfer` の上に載るのでホストでもビルド・試験する
src/fwxfer_kos.c         ATAPI PACKET を G1 レジスタで直接（`fw_xfer`、60 秒ウォッチドッグの計上つき）  [KOS 専用]
src/ngblk.c              書き込みごとの FNV-1a 照合と書き直し
src/diskio.c             FatFs の diskio
src/ngfs.c               FAT 操作（確保、チェックポイント、確定 ngfs_commit_ext、削除、一覧、probe）
src/recv_core.c          put / verify / delete / list / finish の状態機械（転送路に依存しない）
src/xport_pc.c           T1：job.ngj、/pc/ からの読み出し、NGD: 行、result.ngr、reload/cancel
src/manifest.c crc32.c   マニフェスト、CRC32/FNV-1a
third_party/fatfs/       FatFs R0.15b（ChaN、ライセンス表記つき。ffconf.h だけ変更）
test/                    ホスト試験（dev_file.c = ファイルを SD に見立てる、fwctl_sim.c = ファームの**パケット単位の**模型（`fw_xfer`。UNIT ATTENTION を出せる）、run_tests.py、fatcheck.py）
```

ファームのパケットの形はすべて `src/fwctl.c` の 1 ファイルにある（ファーム側の実装 `mcu/opengdemu/src/ide.rs` と `common/discimg/src/wsession.rs` に合わせた）。

## テスト（ハードウェア無し）

```
python tools\netgd-recv\test\run_tests.py               # 全部（gcc が要る）。電源断の総当たりは「速い方」
python tools\netgd-recv\test\run_tests.py --full-sweep  # 電源断を全位置で（NETGD_SWEEP=full でも同じ。CI=true のときは既定でこちら）
python tools\netgd-recv\test\run_tests.py TestBasics    # 動作確認だけ
```

- **所要時間の目安**（29 件、2026-11-20 の計測）：Linux（WSL、gcc 13）で 約 10 秒（全位置で 約 13 秒）。Windows（MSYS2 gcc、作業フォルダは内蔵 SSD の一時フォルダ、他のビルドで負荷が高い状態）で 約 30〜60 秒（全位置で 約 55〜60 秒）。
- **電源断の総当たりの 2 つの方式**：「速い方」（既定）は、メタデータを書く区間（フォルダ・`NETGD.MF`・`NETGD.CKP`・確保の全書き込み、各ファイルの終わりの前後、読み戻しから確定・`NETGD.CKP` の削除まで）は**全位置**で切り、単なるデータセクタの区間は十数か所だけ切る。`--full-sweep` は全位置（「medium」だけは 13 書き込みごと）。
- **時間切れは失敗にする**（止まったままにしない）：子プロセス 1 つにつき `NETGD_CHILD_TIMEOUT` 秒（既定 120）、テスト 1 件につき `NETGD_TEST_TIMEOUT` 秒（既定 1800。総当たりはそこで失敗として止まり、それ以外で 60 秒過ぎても終わらなければスタックを出して異常終了）。
- **作業フォルダはシステムの一時フォルダ**（`ngr-xxxx`、終了時に消す）。`NETGD_TEST_WORK=<フォルダ>` で変更できる（空白・非 ASCII を含まないパス。`/pc/` の規則）。**遅いディスクに置かないこと**：模擬 SD への書き込みはセクタごとにファイルへ flush し、1 回の実行で千回以上、短い子プロセスを起動する。以前は `test/_work_*`（ソースと同じドライブ）に置いていて、リポジトリが USB の HDD にあると 1 回の切断点に 3〜5 秒かかり、総当たり 1 件が数分〜十数分になって「固まった」ように見えた（R48 の修正中に実際に起きた。中核の無限ループではない）。

`gcc` でアプリの中核（マニフェスト、チェックポイントと再開、確定、`result.ngr`、FatFs 経由の FAT 層）をビルドし、FatFs の `f_mkfs` で MBR 付き FAT32 イメージを作って、`netgd-send prepare` で作った本物のジョブフォルダ（`/pc/` はホストのフォルダに置き換え）を実行する。確認する内容：全ファイルの内容とパーティション外への書き込み無し、連続確保、`.NGP` と `NETGD.CKP` が残らない、`netgd-send collect` が終了コード 0 / 2 を返し再開位置の CRC も合う、FatFs で読み戻した CRC、既存フォルダの扱い（new / resume / overwrite / 保護）、ENOSPC で SD が 1 バイトも変わらない、断片化（EFRAG と `fragok.req`）、G1 のビット化けの模擬（書き込みの書き直し、**1 セクタ読みのビット化けの再読**）、途中停止と再開（**読み戻し中の中断**を含む）、UNIT ATTENTION（2 回続けて実行・END 2 失敗後の `MOUNT 0xFF`）、BEGIN 後のエラーでも END が出ること、フォルダ名の検査と保護フォルダの正規化、FSInfo が古いカード（漏れたクラスタ）で空き容量を偽らないこと、チェックポイントが「連続」と偽った断片化ファイル、`list` の 32 件超、**電源断の総当たり**（書き込みセクタ数 N = 0 から最後まで。どの N でも独立した FAT リーダ `fatcheck.py` で構造の破損が無く、見えているイメージは古いか新しい完全なものだけ、再開すると完走して取りこぼしクラスタは確保中の分だけ）。

## 未実装・今後

- T2（NGTP、単体 TCP）、`dcload_type != IP` のときの起動は未実装（その旨を表示して終了）。
- `fsck.fat`（Windows には無い）の代わりに、`test/fatcheck.py`（FAT32 の独立した読み取り実装）で検査している。
- 実機でしか確かめられないこと：下のリスト。

## KOS 専用部分の初回確認リスト（実機・KOS ビルドで最初に見る所）

1. **コンパイル**：`main.c`（`bfont_draw_str` の引数と `vram_s`、`KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` の書き方、`dcload_type`）、`plat_kos.c`（`cont_state_t`、`maple_dev_status`）、`dev_kos.c`（`memalign`、`thd_create`、`arch_abort`、`timer_ms_gettime64`）、`fwctl.c`（`g1_ata_mutex_lock/unlock` と `g1_ata_select_device` が KOS 2.3.0 のヘッダで公開されているか。設計 2.3 で「要確認」とした点）。KOS 2.3.0 の `g1ata.h` にあることは確認したが、ビルドはしていない。
2. **`INIT_CDROM` を外してよいか**：外す理由は、セッション中に KOS が GD-ROM に触らないようにするため。外すと起動時の GD-ROM 初期化が無くなる。dcload-ip からの起動で問題が出たら外す。
3. **ATAPI の直接発行**（`fwxfer_kos.c` の `fw_xfer()`。パケットの形と UNIT ATTENTION の再送は `fwctl.c`）：プライマリ（GD-ROM）への PACKET コマンドを G1 の ATA レジスタで直接出している。BSY/DRQ の待ち方、バイトカウント、データ読み出し。VERSION が `OGDM` を返すかが最初の確認。KOS の GD-ROM ドライバや BIOS が同じバスを同時に使わないこと。
4. **セカンダリの検出と容量**：`g1_ata_init` が成功し、`g1_ata_blockdev_for_device(0, …)->count_blocks` が SD の全セクタ数を返すか（`dev_data` を `free` している。KOS 2.3.0 で形が同じか）。
5. **DMA**：ファームの VERSION 機能 bit2 が立つとき `g1_ata_*_lba_dma` を使う。マウント時に -1 が返れば自動で PIO に落ちる（`dev_kos.c`）。DMA が一度でも成功した後の失敗はそのままエラー。バッファは 32 バイト境界（アプリは `iobuf` と `bounce` を整列済み）。
6. **`g1_ata_flush`（FLUSH CACHE）が成功するか**（ファーム F2）。失敗すると FatFs の `CTRL_SYNC` が `FR_DISK_ERR` になる。
7. **STATUS の FNV 照合が通るか**：`NGD:HELLO caps=7` のとき `ngblk_stats.fnv_mismatches` が 0 のままか。最初の実行で `fnvbad` が出るなら、ファームの FNV の取り方（コマンド全体で 1 つの値）とアプリの照合（`ngblk_write`）を突き合わせる。
8. **`/pc/` の読み出し**：64 KiB 単位の `fread`、再開位置の `fseek`（2 GiB を超える位置は読み捨て）。読み出し速度を測る（設計 G2-3b）。
9. **セッション終了後に BIOS がディスクを認識するか**（設計 R10）：`WSESSION_END` の後 `exit(0)` で dcload に戻り、さらに本体メニューへ戻れるか。
10. **`localtime`/`time`**（`get_fattime`）が RTC を返すか（返さなくても動くが、ファイルの日付が 2025-06-01 になる）。
11. **スタック**：`ng_probe_t`、`FILINFO`（LFN 255 文字）、`list_rec` の 4 KB 配列が KOS のメインスレッドで足りるか（足りなければ `static` にする）。
12. **BBA の速度と dcload の呼び出し**：1 秒に 1 回の `reload.req`/`cancel.req` の確認が転送を邪魔しないか。
