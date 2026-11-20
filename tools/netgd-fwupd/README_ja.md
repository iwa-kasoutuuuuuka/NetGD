# netgd-fwupd：Dreamcast から NetGD のファームを更新する

Dreamcast（KallistiOS 2.3.0）上で動き、PC の `/pc/`（dcload-ip）にある更新イメージ（`tools/fwimage` の OGFW）を読み、
**GDEMU のセカンダリ IDE デバイス**へ独自コマンド `0xF9` で送って、NetGD 自身のフラッシュを書き換える。
受け側は `mcu/netgd/src/fwupdate.rs`（入口）と `mcu/fwupdater`（RAM 上の書き込みプログラム）。
形式と手順は `tools/fwimage/FORMAT.md`、安全の考え方は `docs/research/LOG.md` R37・R46。

> **実機では一度も動かしていない。** KOS のビルドも未確認（この PC に KOS が無い。CI の `kos.yaml` が確かめる）。
> 更新に失敗すると基板が起動しなくなり、**SWD でしか戻せない**ことがある（下の「安全のルール」）。
> 初回は必ず末尾の「初回チェックリスト」に従うこと。

## やること

1. `/pc/` から `.ogfw` を 8 KB ずつ読み、コンソール上で検査する（マジック、形式 2、ヘッダ CRC、ペイロード CRC、
   書き込み先と長さ、コードのベクタテーブル、複数画像の整合）。`fwimage.py inspect` と更新プログラムの検査と同じ規則。
2. セカンダリ IDE に `IDENTIFY DEVICE` を送り、型番文字列が `OpenGDEMU` または `NetGD` を含むときだけ先へ進む
   （他の機器にはベンダーコマンドを一切送らない。純正 GDEMU には送らない）。
3. 確認画面（対象チップ、各画像の種別・大きさ・版・CRC、警告）。**A + X + START + 十字キー上を 3 秒間押し続ける**と始まる。
   B で中止（何も送らない）。この時点まで `0xF9` は 1 回も送っていない。
4. ハンドオーバー、各画像の送信、検証、全部成功したときだけリセット要求。
5. 画面に結果を日本語で出し、`/pc/` に結果ファイルを書く。

対象チップ（SAM3U2E / 4E）は画像のヘッダが持っている CHIPID から表示する。**Dreamcast からは基板の CHIPID を読めない**ので、
基板用かどうかの最終判断は更新プログラムの `BadTarget`（`0x84`）に任せ、それを画面と結果に出す（何も書かれる前に返る）。

## 安全のルール

- **転送中は電源を切らない・リセットしない・LAN ケーブルを抜かない。** ページ 1 以降を消してからページ 0（ベクタテーブル）を
  書き直すまで（転送中ずっと）と、ページ 0 の書き込みの数 ms は、電源が落ちると**基板が起動しなくなり SWD が必要**になる（R37・R46、
  `FORMAT.md` 末尾）。常駐ブート領域（R37 対策 B）が入るまで解消しない。2E で使う前に SWD の手段（Pico など、`fwbackup` の手順）を用意すること。
- **このアプリは、全画像の検証が成功した後の `F9/58`（リセット）以外のリセットを送らない。** 強制リセット（`F9/59` とその鍵）は
  コードに定数すら無い（ホストテストが「どの実行でも `0x59` を送らない」ことを確かめる）。失敗しても自分でリセットしない。
- 失敗したら、**状態によって正しい対処が逆になる**：
  - フラッシュを書く前に拒否された（`BadTarget`・`Locked`・`BadVector`・`BadHeader`…）：何も書き換わっていない。基板は更新プログラムの
    待機中なので、**電源を入れ直せば元のファームで起動する**。
  - 書き込み中・検証中に失敗した、または応答がなくなった：フラッシュは途中。**電源を切らず**、同じ LAN・同じ Dreamcast のまま
    **このアプリをもう一度実行する**（更新プログラムは RAM で動き続けている）。アプリは `arm` が拒否されたとき、`F9/57` で
    更新プログラムが動作中かを確かめ、動作中なら続きからヘッダを送り直す。電源を切るのは、何度やっても通らず、SWD の用意ができてから。
- 画像ごとの自動再送は、`Overrun`・`FlashError`・`CrcMismatch`・`NoHeader`・`BadLength` のときだけ最大 2 回（ヘッダからやり直す）。
  `BadTarget`・`BadVector`・`Locked`・`BadHeader`・`BadDest`・タイムアウトは再送しない。
- 書き込みセッション中（`netgd-recv` の途中）は NetGD が `0xF9` を拒否する（`ide.rs` `handle_fw_command`）。そのときは何も変わらず
  「更新を開始できませんでした」と出る。

## 前提条件

- NetGD の**ファームが `0xF9` の入口と R46 の対策 A を含んでいる**こと（`fwupdate.rs`、`mcu/fwupdater`）。古いファームでは `arm` が拒否される。
- Dreamcast + BBA（HIT-0400）、PC 側は dc-tool-ip（dcload-ip）。プログラムは **dcload から起動したときだけ**動く（`/pc/` が要る）。
  **`/pc/` はそのドライブのルート**。更新イメージ・結果ファイルのフォルダは先に作っておく。
- 書き込みセッション中でないこと。GD-ROM（プライマリ）には触らない（`INIT_CDROM` を外してある）が、ディスクを排出している必要は無い。
- 更新するファームの ELF が手元にあること（`cargo build --release` の成果物。ビルドのたびに上書きされるので別名で保存）。

## 更新イメージを作る

```powershell
# 2E（コードのみ）：
python tools\fwimage\fwimage.py build --elf <netgd の ELF> --target 2e -o fw_2e.ogfw --image-version 12
# 4E（コード + ビットストリーム）：--target 4e
python tools\fwimage\fwimage.py inspect fw_2e.ogfw          # 検査（CRC・ベクタ）
```

更新イメージは 1 〜 2 個の画像（コード、4E ではビットストリームも）をつないだファイル。コードは `0x80000`、ビットストリームは `0x100000` へ。
送る順序はファイルの並び（`fwimage build` はコード → ビットストリーム）。

## ビルド

```powershell
$env:BBA_TOOLS = 'H:\DC\tools\dc-bba-tools'                # Dreamcast-Dev-Tools-for-BBA の場所
.\tools\netgd-fwupd\build.ps1 -Image fw_2e.ogfw            # netgd-fwupd.elf を作り、host\netgd.ogfw に画像を置く
```

DreamSDK のシェルなら素の `make`：

```sh
cd tools/netgd-fwupd
make HOST_DIR_REL=/work/OpenGDEmu/tools/netgd-fwupd/host   # /pc/ から見たフォルダ。スペース不可
make NG_FWIMAGE=/pc/work/x/netgd.ogfw                       # 画像のパスを焼き込む場合
```

画像のパスは、`<HOST_DIR>/netgd.ogfw`（既定）、ビルド時の `NG_FWIMAGE`、実行時の `<HOST_DIR>/netgd-fwupd.cfg` の 1 行
`image=/pc/...`（`#` 以降はコメント）のうち、**後ろのものが優先**（cfg が最優先）。`.github/workflows/kos.yaml` は
`make HOST_DIR_REL=/ci` で CI 上でもビルドする。

## 実行

```powershell
cd $env:BBA_TOOLS
.\scripts\dc_run.ps1 E:\OpenGDEmu\tools\netgd-fwupd\netgd-fwupd.elf     # 1 回だけ送る
```

**`dc_auto` のプリセットは意図的に用意していない**（ファイル監視で自動的に再送される仕組みは、ファーム更新アプリには向かない）。
`dc_run` で送ったあと、画面の指示に従う。dc-tool-ip は、このリポジトリと同じドライブに current directory を置いて動かす。

### 画面

1. 検査と接続先の確認（NetGD でなければここで止まる）。
2. 確認画面：接続先の型番、ファイル、対象チップ、画像ごとの種別・バイト数・版・CRC、警告。**3 秒押し続け**の進み具合が出る。途中で離すとやり直し。
3. 更新中：手順名、`画像 n/m`、進み具合（512 バイトブロック）、再送中の表示、電源を切るなの警告。キャンセルは無い。
4. 結果：日本語の説明、失敗した画像とステータスコード、結果ファイルの名前。START で dcload に戻る。

### 結果ファイル

`<HOST_DIR>\fwupd_result_NNN.txt`（NNN は 001 から、空いている最初の番号。UTF-8）。4 KB を超えない塊で書き、1 回ごとに `fflush` する
（`KNOWHOW_ja.md` 4 章）。更新の途中で落ちても、そこまでの行は残る。

```
# netgd-fwupd result
file=/pc/.../netgd.ogfw size=119296 images=1 target=SAM3U2E
image0 kind=code dest=0x00080000 len=118784 crc=0x8743df86 cidr=0x281a0761 version=12
t=1234 step=arm image=-1 attempt=0 feat=0x4f blocks=0 status=0x50 error=0x00 ok
...
t=5678 step=header image=0 attempt=0 feat=0x55 blocks=1 status=0x51 error=0x84 name=BadTarget ja=このイメージはこの基板（チップ）用ではありません。
outcome=rejected handed_over=1 resumed=0 flash_touched=0 images_verified=0 retries=0 failed_image=0 status=0x51 error=0x84 name=BadTarget
message_en=...
message_ja=...
```

## 送る手順（実装したもの）

`mcu/netgd/src/fwupdate.rs` と `mcu/fwupdater/src/main.rs`、`FORMAT.md`「Sending」のとおり。すべて `0xF9`、セカンダリ（`DEVICE_SELECT = 0xF0`）、
PIO。FEATURES = 手順、LBA = 48 ビット鍵、セクタ数 = ブロック数（0 = 256）。

| 手順 | feat | 鍵 / データ | 根拠 |
|---|---|---|---|
| arm | `0x4F` | LBA48 = `0x4F4744555041` | `fwupdate.rs:41,45,100`、`ide.rs`（HEAD）`handle_fw_command` 3133-3162 |
| enter | `0x53` | LBA48 = `0x4744454D5521`。一方通行（更新プログラムが RAM で動き出す） | `fwupdate.rs:42,48,115`、`ide.rs` 3144 |
| （probe） | `0x57` | arm が拒否されたとき 1 回だけ。更新プログラムが動作中か（通常ファーム = 素の `0x04`、更新プログラム = `0x?4` か成功） | `main.rs:425,578` |
| header | `0x55` | 512 バイト 1 ブロック | `main.rs:415,462` |
| data | `0x56` | 512 × n バイト（n ≤ 64 で送る。1〜256 が可能、256 は count = 0）。DRQ ごとに 1 ブロック | `main.rs:419,505` |
| verify | `0x57` | なし。全体 CRC → ページ 0 を書く → フラッシュを CRC し直す | `main.rs:425,578` |
| reset | `0x58` | 全画像の verify 成功後に 1 回だけ。更新プログラムは検証前なら `NotVerified`（`0xB4`）で拒否 | `main.rs:429` |

LBA48 は `LBA_LOW/MID/HIGH` に「ビット 24〜47」を先に、「ビット 0〜23」を後に書く（KOS `g1ata.c` と同じ順。FPGA のレジスタは
スライディングウィンドウ：`ide_device.py` 642-663、`ide.rs` `lba48()`）。

ステータス（ERR ビット + ERROR レジスタ）は `main.rs:132-151`：`0x14` BadHeader、`0x24` BadDest、`0x34` BadLength、`0x44` NoHeader、
`0x54` Overrun、`0x64` FlashError、`0x74` CrcMismatch、`0x84` BadTarget、`0x94` Locked、`0xA4` BadVector、`0xB4` NotVerified、
素の `0x04` = 通常ファームの拒否。すべて日本語の説明を付けて画面と結果に出す（`fwmsg.def`）。

タイムアウトは、arm・enter・header・reset が 5 秒、data の 1 回の待ちが 10 秒、verify が 60 秒（`fw_options_default`）。待ちはすべて上限付き。

## G1 バスの触り方（`g1dev.c`）

- `_g1_ata_sem` を直接取る（`gdprobe/gdraw.c` と同じ）。`g1_ata_mutex_unlock` は `DEVICE_SELECT` を書き戻すので使わない。
- コマンドごとに `DEVICE_SELECT = 0xF0`（スレーブ + LBA）を明示的に書く。BSY/DRQ が下がるのを待ってから。終了時に元の値へ戻す。
- `g1_ata_init` は呼ばない（DMA ハンドラの設置を避ける）。KOS の `g1_ata_scan` が `IDENTIFY` の後にやる 2 つ（`SET FEATURES` の PIO 既定モード、
  PIO アクセスウェイト `0x222`）だけを、NetGD と確認できた後に行う（`prepare`。失敗しても続行して記録に残す）。
- `KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)`。

## ホストテスト

```sh
sh tools/netgd-fwupd/tests/run_tests.sh          # gcc（と、あれば python 3）。Windows は Git Bash で
FWIMAGE_ELF_2E=<elf> FWIMAGE_ELF_4E=<elf> sh tools/netgd-fwupd/tests/run_tests.sh   # 実 ELF から作った画像も通す
```

- `fwcore.c`（画像の検査、`0xF9` の手順、結果テキスト）を、`tests/fakedev.h` の**偽の NetGD**（通常ファームの `F9` 処理と RAM 上の更新プログラムを
  Rust から書き写したもの）につないで試す：成功（2E 1 画像、4E 2 画像、`count = 0` = 256 ブロック、1 ブロックずつ）、BadTarget、BadVector、
  Locked、CRC 不一致（1 回だけ → 再送で成功、ずっと → 失敗で終了しリセットしない）、フラッシュ書き込みエラー、Overrun、手順ごとのタイムアウト、
  reset の拒否・無応答、書き込みセッション中の拒否、前回の続き（更新プログラムが動作中）、IDENTIFY の判定、
  どの実行でも `0x59` を送らないこと、鍵と feat の値。
- `tools/fwimage/vectors/*.txt`（CRC・ベクタ・ヘッダ）をそのまま読んで確かめる。`tests/gen_golden.py` が `fwimage.py` で作った画像
  （2E、4E、`FWIMAGE_ELF_*` があれば実 ELF）を C 側で読んで偽の更新プログラムに通し、Python と C の食い違いを検出する。
- 日本語メッセージ（`fwmsg.def` → `gen_msgs.py` → `fwmsg_gen.h`、UTF-8 と EUC-JP）の折り返しと、画面の行数に収まること、`fwmsg_gen.h` が最新であること。
- 結果ファイルの書き込みが 4096 バイトを超えず、毎回 flush すること。

KOS 側（`main.c`、`g1dev.c`）はスタブの `kos.h` で `gcc -fsyntax-only` を通しただけ。

## 実機で未確認のこと

- **全部**。特に：(a) arm・enter・data の IDE 手順が FPGA 実機で通るか（PIO の待ち、`DRQ`/`BSY` の動き、スライディングウィンドウ）。
  (b) `enter` の後の最初のステータス（更新プログラムの `0x50`）を `BSY` 解除で拾えるか。(c) 偽の更新プログラムは Rust からの手移植なので、
  実物との差があり得る。(d) `reset` の直後にバスが静かになる挙動（無応答は「リセットした可能性が高い」と扱う）。
  (e) `SET FEATURES`・PIO ウェイトの設定が NetGD の IDE と相性が良いか。(f) `bfont` の EUC-JP 表示（BIOS フォントの日本語、1 行 50 セル）。
  (g) コントローラーの組み合わせ入力。(h) probe（`F9/57`）の判定。通常ファームは素の `0x04`、更新プログラムは `0x?4`（`?` ≠ 0）か成功、という前提。
- 4E のビットストリーム（プレーン 1）の書き込み、EEFC1。
- ページ 0 の書き込みの数 ms と転送中の電源断は、このアプリでは防げない（R37 対策 B 待ち）。

## 初回チェックリスト

1. SWD の手段（Pico の配線は `docs/research/LOG.md` R44）を用意し、基板に SWD で届くこと（`tools/fwbackup` の `check`）を先に確かめておく。
2. **安定した電源**で、Dreamcast・NetGD・LAN を触らない（電源タップやケーブルに足を引っかけない）。
3. `fwimage.py inspect` が OK、`--target` が基板と同じ（2E/4E）。ELF は検証済みのビルド。
4. まず**確認画面で B（中止）**まで試す：画像の検査と IDENTIFY が通り、`0xF9` が 1 回も送られない（結果ファイルに `step=` の行が無い）。
5. 次に**わざと対象違いの画像**（2E 基板に 4E 用）で実行：`BadTarget` で止まり、フラッシュが変わらず、電源を入れ直すと元のファームで起動する。
6. 本番：2E なら**コードのみの小さな画像（変更なしの同じファーム）**で最初に試す。成功したら、基板が再起動して Dreamcast から見えること、
   結果ファイルの `outcome=ok` を確認する。
7. うまくいかなかったら、電源を切る前に**結果ファイルの全文と、dcload のコンソールのログ**を残す。

## ファイル

| ファイル | 中身 |
|---|---|
| `main.c` | KOS の画面・パッド・ファイル・流れ（KOS 専用） |
| `g1dev.c/.h` | `/pc/` と別の、G1 バスの生アクセス（KOS 専用） |
| `fwcore.c/.h` | 純粋な核（画像の検査、`0xF9` の手順、結果テキスト）。ホストでテスト |
| `fwmsg.c`、`fwmsg.def`、`gen_msgs.py`、`fwmsg_gen.h` | 日本語メッセージ（`make msgs` で再生成、生成物はコミット） |
| `resout.c/.h` | 4 KB 単位 + flush の結果ファイル書き込み（`gdprobe/pcout.c` と同じ） |
| `tests/` | `run_tests.sh`、`test_all.c`、`fakedev.h`、`gen_golden.py` |
| `Makefile`、`build.ps1` | KOS ビルド（`kos.yaml` が使う）、DreamSDK からのビルド |
