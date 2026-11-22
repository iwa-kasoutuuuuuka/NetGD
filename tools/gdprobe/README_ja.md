# gdprobe：GD-ROM ドライブの読み取り専用プローブ

Dreamcast（KallistiOS 2.3.0）上で動き、GD-ROM ドライブ（= **純正ファームの GDEMU 6.0** または **NetGD**）に
**読み取り系のコマンドだけ**を送って、生の応答バイトと所要時間を記録し、BBA 経由（dcload-ip の `/pc/`）で PC に保存する。
同じ試験を純正と NetGD の両方で走らせ、PC 側で比べる（`compare.py`）。

**実機での使用実績**：純正 GDEMU 6.0 で 11 回実行済み（2026-11-21〜22）。記録は `docs/research/probe-records/`（`stock_20261121_01`〜`stock_20261122_11`）、結果は `docs/research/LOG.md` の R117〜R135。

調べたいこと（`docs/research/LOG.md` R17・R11・R10・R13、`docs/research/stock-gdemu-flow.md`）：

| 知りたいこと | 記録のどこか |
|---|---|
| NetGD の ATAPI 応答を純正に合わせるべきか（`docs/PLAN.md` D-1〜D-5） | `REQ_SES_*`、`GET_TOC_*`、`GET_SCD_*`、`REQ_STAT`、`REQ_ERROR`、`REQ_MODE_*` の `data=` |
| `0x52` の実際の挙動（バージョン、切替後の SENSE の並びと READY までの時間） | `GDEMU_VERSION`、`SW_CMD` / `SW_POLL` / `SW_RESULT` |
| 空フォルダ（07）へ切り替えたとき純正がどう応答するか | `img=7` の `SW_*` 行と、その後の情報コマンド |
| 性能の基準値 | 各行の `us=`、`TP_SUMMARY`（CD_READ スループット）、`SEEK_*` |
| データの完全性 | `HASH` 行（セクタごとの CRC-32）→ `compare.py --image` |

## 安全のルール（正直な説明）

- **送るコマンドは1つの許可リスト（`whitelist.c` の `wl_table`）にあるものだけ**。ランナーは、リストに無い CDB を「`rc=refused`」と記録して**送らない**
  （ホストテストで、許可リスト外のオペコード 0〜255 と、CDB のバイトを乱数で振った 30 万件が通らないことを確認済み）。

  | コマンド | オペコード | 何をするか |
  |---|---|---|
  | TEST UNIT READY | 0x00 | 準備できているかを聞くだけ |
  | REQ_STAT / REQ_MODE / REQ_ERROR | 0x10 / 0x11 / 0x13 | 状態・モードデータ・エラー情報を読む（REQ_MODE は SET_MODE ではない）。REQ_ERROR は 10 バイトで、バイト 2 の下位 4 ビット = センスキー、バイト 8 = ASC、バイト 9 = ASCQ。GD-ROM には ATAPI の REQUEST SENSE（0x03）が無い（SPI 仕様 Table 6.1）ので、これがセンスの読み出しに当たる |
  | GET_TOC | 0x14 | TOC を読む（SD 領域・HD 領域の両方） |
  | REQ_SES | 0x15 | セッション 0〜3 の開始位置を読む |
  | GET_SCD | 0x40 | サブコード Q（形式 0・1）を読む |
  | CD_READ | 0x30 | データセクタを読む（FAD 指定、1 回 64 セクタまで）。バイト1 は `0x20` / `0x24` / `0x28` / `0x10` と設定の `cdread_flags` の値だけ許可（`0x30`・`0xE6`・`0xF0`、MSF 指定の `0x11` などは拒否） |
  | GDEMU バージョン | `52 00` | 8 バイトのバージョンを読む |
  | **GDEMU 切替（オプトイン）** | `52 82 n` | フォルダ n のイメージをマウントする。openMenu がゲームを起動するときと同じ。設定で `switch=on` のときだけ許可 |
  | IDENTIFY PACKET DEVICE（オプトイン） | ATA 0xA1 | 機器の識別文字列（GD-ROM は 80 バイトを返す）。設定で `identify=on` のときだけ |

- **送らないもの**：SET_MODE（0x12）、CD_OPEN（0x16）、CD_PLAY / SEEK / SCAN、書き込み・フォーマット系、`0x70` / `0x71`（セキュリティ）、
  その他すべてのベンダーコマンド、`0x52` の未知のサブコード（`81` 次／前も送らない）、セカンダリの IDE デバイスへの送信
  （デバイス選択レジスタには書かない）。
- **リスクについて**：どれも「読む」か「openMenu が普通に送る切替」だけなので、純正ファームを壊す見込みは低い。
  ただし純正ファームは中身が分からず、**「絶対に安全」とは言えない**。特に初めて純正に対して実行するときは、
  切替なし（`switch=off`）で1回走らせてから、切替を有効にすること。壊れて困る基板に対して、という意味での「やってはいけない」は無い
  （書き込み・ファーム更新に当たるコマンドは許可リストに無い）が、**SWD 端子・ERASE パッドには絶対に触らない**（純正が消える）。
- すべての待ちにタイムアウトがあり、3 回続けて応答が無ければ記録に `ABORT` を書いて終了する（無限ループしない）。
  タイムアウト後のドライブの状態を戻すコマンドは送らない。止まったままなら Dreamcast の電源を入れ直す。

## ビルド（DreamSDK）

KOS 2.3.0（ユーザーのフォーク KallistiOS-VisualNovel）に合わせてある。この PC には KOS が無く、**KOS に依存する部分
（`gdraw.c`、`main.c`）はコンパイルしていない**（スタブの `kos.h` での構文チェックだけ）。初回のビルドエラーは全文をそのまま渡すこと。

### Dreamcast-Dev-Tools-for-BBA の流れ（推奨）

```powershell
$env:BBA_TOOLS = 'H:\DC\tools\dc-bba-tools'      # Dreamcast-Dev-Tools-for-BBA の場所
.\tools\gdprobe\build.ps1                         # gdprobe.elf と host\gdprobe.cfg を作る
```

`build.ps1` は `sample/hello_bba/build.ps1` と同じ作りで、`tools\gdprobe\host\` を `/pc/` 側の保存先として `gdprobe.elf` に埋め込む
（`HOST_DIR_REL`）。**dc-tool-ip は、このリポジトリと同じドライブに current directory を置いて動かす**（`/pc/` はそのドライブのルート）。

### 素の make

DreamSDK のシェルで（`KOS_BASE` が通っていること）：

```sh
cd tools/gdprobe
make HOST_DIR_REL=/work/OpenGDEmu/tools/gdprobe/host   # /pc/ から見たフォルダ。スペース不可
make run DC_IP=192.168.10.2                              # または dc-tool-ip -t <IP> -x gdprobe.elf
```

`HOST_DIR_REL` を省くと保存先は `/pc/`（ドライブのルート）になる。**保存先のフォルダは先に作っておく**（dcload は作れない）。

## 実行

1. SD カードを用意し（`docs/research/stock-gdemu-flow.md` ③）、Dreamcast を起動、`02` の dcload-ip ディスクを選ぶ。
2. 設定ファイル `host\gdprobe.cfg` を書く（下記。最初は `label=` だけ、`switch=off`）。
3. `dc_auto` のプリセットで実行：

   ```powershell
   cd $env:BBA_TOOLS
   .\scripts\dc_auto.ps1 -Preset E:\OpenGDEmu\tools\gdprobe\profiles\gdprobe.ps1
   ```

   （`-Preset` にはプリセットファイルのフルパスを渡す。`profiles\gdprobe.ps1` を BBA_TOOLS の `profiles\` へコピーする方法は**使わない**：
   プリセットは自分の置き場所から gdprobe のフォルダを探すため、コピーすると見つからなくなる。）
   または `dc_run.ps1` で `gdprobe.elf` を直接送る。プログラムは終わると `exit(0)` で dcload に戻る（`dc_auto` が終了を検出する）。
4. コンソールに `[gdprobe] ...` の進捗と、各プローブの行（先頭 150 文字）が出る。記録は
   `host\gdprobe_<label>_<日時>.txt`（Dreamcast に有効な時計が無いときは `seq<番号>`）。

## 設定ファイル（`/pc/.../gdprobe.cfg`）

`key=value` の行、`#` 以降はコメント。未知のキーは無視して数える（打ち間違いで何かが有効になることは無い）。
`gdprobe.cfg.example` が雛形。

| キー | 既定 | 意味 |
|---|---|---|
| `label` | `unknown` | 機器名（`stock` / `netgd`）。ファイル名と記録のヘッダに入る |
| `seq` | 0 | 実行番号。時計が無いときのファイル名に使う |
| `switch` | `off` | `on` で切替ループを実行 |
| `images` | なし | 訪れるフォルダ番号（例 `3,4,5,6,7`、範囲 `3-7` も可、1〜999） |
| `return_to` | 0 | 最後にこのフォルダへ切り替えて戻す（例 `2` = dcload-ip のディスク）。0 = 戻さない |
| `switch_timeout_ms` | 15000 | 切替後 READY を待つ上限（1000〜60000） |
| `identify` | `off` | IDENTIFY PACKET DEVICE を 1 回送る |
| `throughput` | `on` | CD_READ のスループット試験（ハッシュ付き） |
| `tp_batches` / `tp_sectors` | 64 / 16 | バッチ数 / 1 回のセクタ数（1〜16）。既定は 64 × 16 = 1024 セクタ（2 MB） |
| `seek` / `seek_n` / `seek_span` | `on` / 16 / 50000 | シーク試験：IP.BIN 領域から `seek_span` セクタの範囲に `seek_n` 点（遠・近を交互に） |
| `ip_fad` | 0 | 読み始めの FAD。0 = 自動（REQ_SES(2) の値、取れなければ 45150） |
| `cdread_flags` | `0x24` | CD_READ の CDB バイト1（bit 0 = MSF は常に 0 に強制）。許可リストは `0x20` `0x24` `0x28` `0x10` に加えてこの値だけ通す |
| `cdread_flags_test` | `on` | `CDREAD_FLAGS` 試験（下記）。`off` で省略 |
| `timeout_ms` | 3000 | コマンド 1 つのタイムアウト（読み出しは +2000 ms） |

### 実行の流れ

- **`switch=off`**（既定）：いまマウントされているイメージだけを調べる（記録の `img=0`）。
- **`switch=on` と `images=...`**：
  1. 起動時のイメージ（dcload-ip のディスク）の情報コマンドだけを `img=0` で記録（読み出し試験はしない）。
  2. `images` の各番号 n について：`52 82 n` を送り、**20 ms ごとに TEST UNIT READY**（失敗なら REQ_ERROR）を送って
     状態が変わるたびに `SW_POLL` を記録、READY になるか `switch_timeout_ms` で `SW_RESULT n=.. rc=ok|timeout t_ms=..` を記録する。
     そのあと**読み取り系の試験一式**を実行し、**すべての行に `img=n`** を付ける。
  3. 切替が失敗・タイムアウトしても**記録して次へ進む**（07 のような空フォルダの純正の応答を残すのが目的）。
     READY にならなかったイメージでは情報コマンド（TUR・REQ_ERROR・REQ_STAT・TOC・REQ_SES 等）だけを記録し、読み出し試験は飛ばす。
  4. 最後に `return_to` が有効ならそのフォルダへ切り替える（`SW_*` が `img=<return_to>` で残る）。

### 1 つのイメージごとの試験一式

`TUR_W*` / `REQ_ERROR_W*`（UNIT ATTENTION を抜けるまで最大 8 回）→ `TUR`、`REQ_STAT`、`REQ_MODE_0_10`・`_18_8`・`_0_32`、
`REQ_ERROR`、`GET_TOC_0`・`_1`（408 バイト）、`REQ_SES_0〜3`、`GET_SCD_0`・`_1`、`GDEMU_VERSION`（`52 00`）、
`IP_READ`（IP.BIN の 7 セクタ）、`CDREAD_FLAGS`（4 行。下記）、`TP_000〜`（スループット）、`TP_SUMMARY`、`SEEK_00〜`。

### CD_READ フラグ試験（`CDREAD_FLAGS`）

イメージごとに、`IP_READ` と同じ 1 セクタ（IP.BIN の先頭の FAD。`ip_fad` / REQ_SES(2) / TOC の自動判定と同じ値）を、
CD_READ の CDB バイト1 を **`0x20`、`0x24`、`0x28`、`0x10`** の順に変えて 1 セクタずつ読む（FAD 指定、件数 1）。
CHECK になっても止めずに次のバリアントへ進む。`cdread_flags_test=off` で省略できる。

バイト1 の意味（SPI 仕様）：bit 7 = Header、bit 6 = Subheader、bit 5 = Data、bit 4 = Other（セクタ全体 2352 バイト）、
bit 3〜1 = 期待セクタ種別、bit 0 = 0 で FAD / 1 で MSF。Flycast では `0x20` / `0x24` / `0x28` が 2048 バイト、`0x10` が 2352 バイト。

```
img=3 probe=CDREAD_FLAGS flags=0x20 fad=45150 count=1 ssz=2048 crc=0badf00d sense=none cdb=302000b05e00000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2048 us=...
img=4 probe=CDREAD_FLAGS flags=0x28 fad=11852 count=1 ssz=- crc=- sense=5/64/00 cdb=302800... rc=check status=0x51 err=0x50 irr=0x03 bytes=0 us=...
img=3 probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=2352 crc=... sense=none cdb=301000... rc=ok ... bytes=2352 us=...
```

- `ssz` = ドライブが返したバイト数（1 セクタなので `bytes` と同じ）、`crc` = そのデータの CRC-32。`rc=ok` 以外は両方 `-`。
- `sense` = CHECK のとき REQ_ERROR（0x13）で読んだ `センスキー/ASC/ASCQ`（`none` = CHECK ではない）。
- `HASH` 行は書かない（`IP_READ` の `HASH` と FAD が重なるため）。代わりにこの行の `crc` を `compare.py` が直接照合する（下記）。

**見るところ（このイメージの種別ごとに、どのフラグ値を受け付け、何バイト返すか）**

1. `rc=ok` のバリアントの一覧。`0x20`（種別不問）と `0x24`（Mode 1）が通り、`0x28`（Mode 2 Form 1）が `rc=check` で
   `sense=5/xx/xx`（ILLEGAL REQUEST。種別不一致のはず）なら、そのドライブは**種別を検査している**。GD-ROM（Mode 1）と CDI（Mode 2 Form 1）のイメージで
   結果が逆になるかを見る（`0x24` が Mode 2 の CDI で CHECK、`0x28` が Mode 1 で CHECK など）。
2. `ssz`：`0x20` / `0x24` / `0x28` が 2048、`0x10` が 2352 なら Flycast と同じ。`0x10` が 2048 や別の値（2336・2340 など）、
   または `0x20` 系が 2352 を返すなら**その差が互換性の論点**。純正と NetGD で `ssz` が違えば、`compare.py` が `ssz` の差として出す。
3. `crc`：同じセクタなので `0x20` / `0x24` / `0x28` は同じ値のはず（違えば、フラグでデータの範囲が変わっている）。
   `0x10` は同じユーザーデータを含む生セクタ（同期 12 + ヘッダ 4 + ユーザー 2048 + EDC/ECC 288）。
4. `0x10` が CHECK なら、そのドライブは生セクタ読み出しを持たない（または種別必須）。

## 記録の形式

プレーンテキスト、1 プローブ 1 行、`key=value` をスペースで区切る。キーの順序は固定。`#` で始まる行はヘッダ／末尾のコメント（`compare.py` は無視）。

```
# gdprobe version=1 kos=2.3.0 label=stock stamp=20261120-143000 seq=0 dcload=1
# config switch=1 identify=0 throughput=1 tp=64x16 seek=1/16 ip_fad=0 cdread_flags=0x24 timeout_ms=3000 return_to=2 images=3,4,5,6,7
img=3 probe=SW_CMD n=3 cdb=528203000000000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=0 us=1500
img=3 probe=SW_POLL t_ms=1 poll=1 rc=check status=0x51 err=0x20 sense=2/04/01
img=3 probe=SW_RESULT n=3 rc=ok t_ms=325 polls=15 changes=3
img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010003 ... us=1234
img=3 probe=IP_READ fad=45150 count=7 cdb=... rc=ok ... bytes=14336 data=<先頭 512 バイト> us=...
img=3 probe=HASH fad=45150 ssz=2048 crc=0badf00d
```

- `rc` = `ok` / `check`（ERR ビットあり）/ `timeout` / `busy` / `proto`（不正なフェーズ）/ `refused`（許可リスト外で未送信）。
- `status` / `err` / `irr` = ATA のステータス、エラー、割り込み理由レジスタ（`err` の上位 4 ビット = センスキー）。
- `bytes` = ドライブが返した実際のバイト数、`data` = その先頭（最大 512 バイト、16 進）。`us` = PACKET コマンドから完了までのマイクロ秒。
- `SW_POLL` の `sense=キー/ASC/ASCQ`（`none` = エラーなし）。変化したときだけ書く。
- `HASH` = 読んだセクタごとの CRC-32（`ssz` = 1 セクタのバイト数。2048 = ユーザーデータ、2352 = 生セクタ）。

## 比較（`compare.py`、Python 3 標準ライブラリのみ）

```
python compare.py stock.txt netgd.txt            # プローブごとの差分（rc / status / err / bytes / data / ハッシュ）。時間は「注記」で別枠
python compare.py --image disc.gdi rec.txt       # GDI のトラック表からセクタを再計算して、HASH 行と照合
python compare.py --image disc.gdi --img 3 rec.txt   # img=3 の HASH だけ
python compare.py --manifest OUTDIR/NETGD-PROBE-MANIFEST.json rec.txt   # tools/mksdcard のイメージ（03〜06）。ファイル不要、期待値を再計算
python compare.py --raw track03.bin --raw-start-fad 45150 rec.txt   # 生のトラックファイル 1 本と照合（CDI から取り出したデータトラックなど）
```

- 終了コード：0 = 差分なし／ハッシュ全一致、1 = 差分あり／不一致、2 = 入力エラー。
- 時間（`us` `t_ms` `kib_s`）は比が 1.5 倍を超えたときに「TIMING」として注記するだけで、差分には数えない（`--ratio`、`--ignore-timing`）。
- 同じ名前の行（`SW_POLL` など）は出現順で対応づける。`CDREAD_FLAGS` だけは `(img, flags)` で対応づける（片方にバリアントが無くても、他の行がずれない）。
  `rc` / `ssz` / `crc` / `sense` の違いが差分として出る。
- `--image` / `--manifest` は `HASH` 行に加えて、データが返った（`rc=ok`）`CDREAD_FLAGS` 行の `crc` も照合する（出力には `flags=0x..` が付く）。
  **`--manifest`**：`tools/mksdcard` の `sector_for` が GD 用 Mode 1 トラックと CDI（XA Form 1）トラックの**生 2352 バイト**
  （同期 + ヘッダ + ユーザーデータ + EDC/ECC）を再構成できるので、`0x10` の行（`ssz=2352`）も照合される。作れないサイズ（2340 など）は「not checked」。
  **`--image`**：2352 バイトのイメージ（GDI の生トラック）なら全体を照合する。2048 バイトのイメージからは 2352 を作れないので「not checked」になる。
- **イメージ照合の前提**：CD_READ の番地は FAD、イメージ側の LBA = FAD − 150。GDI の各トラックの開始 LBA・セクタサイズ・ファイル・オフセットで
  `offset + (LBA − 開始LBA) × セクタサイズ` を読む。`ssz=2048` は、2048 バイトのイメージならそのまま、2352 バイトの生セクタなら
  オフセット 16（Mode 1）または 24（Mode 2 Form 1。15 バイト目のモードで判定）から 2048 バイト。`ssz=2352` は生セクタ全体。
  **Mode 2 Form 2 と CDI の直接読みは未対応**（CDI は、データトラックを `--raw` で渡す）。

### 純正と NetGD の比べ方

1. 同じ SD・同じ設定で、`label=stock` で実行 → `gdprobe_stock_*.txt`。純正で 2 回走らせて、ぶれる項目（時間など）を見分ける。
2. NetGD を書き込んだ後、`label=netgd` で実行 → `gdprobe_netgd_*.txt`。
3. `compare.py stock.txt netgd.txt`。`img=3` の `REQ_SES_*` の `data=` が D-4、`img=4` の `REQ_SES_2` が D-1、`GET_SCD_1` が D-2、`REQ_STAT` が D-3、
   `img=7` の `SW_*` が R18 の欠陥 1 に対応する。
4. `compare.py --image <SD に入れたのと同じ GDI> --img 3 gdprobe_stock_*.txt` で純正のデータ完全性、NetGD の記録でも同じ。

## ファイル

| ファイル | 内容 | ホストテスト |
|---|---|---|
| `main.c` | KOS の入口：設定読み込み、記録の open、ランナー呼び出し、`exit(0)` | なし（構文のみ） |
| `gdraw.c/.h` | G1 の ATA レジスタを直接叩く ATAPI PACKET（タイムアウト付き、`g1_ata_mutex_lock` 下） | **なし（実機のみ）** |
| `probes.c/.h` | プローブ一式と切替ループ（トランスポートは `gd_ops` 経由） | あり（偽のドライブで全体を実行） |
| `whitelist.c/.h` | 許可リスト（唯一の表） | あり（網羅・ファジング） |
| `record.c/.h` | 記録の行の整形 | あり |
| `pcout.c/.h` | `/pc/` への 4 KB 以下の書き込み + `fflush` | あり（モックで確認） |
| `hash.c/.h` | CRC-32（`zlib.crc32` と同じ） | あり |
| `cfg.c/.h` | 設定ファイルの解析 | あり |
| `compare.py` | 比較・イメージ照合 | あり（合成の記録と GDI） |
| `Makefile` / `build.ps1` / `profiles/gdprobe.ps1` / `gdprobe.cfg.example` | ビルド、dc_auto のプリセット、設定の雛形 | — |
| `tests/` | `sh tests/run_tests.sh`（gcc と python が要る。make は不要） | — |

## ホストテストでできていないこと（実機で最初に確認すること）

ホストテストは、偽のドライブ（状態遷移を模したもの）に対するロジックの確認で、**実機の G1 バスの挙動は何も確認していない**。

1. **ビルドが通るか**：`gdraw.c` / `main.c` は KOS のスタブヘッダに対する構文検査だけで、実際のリンクは未確認。KOS 2.3.0 の関数名は fork で確認済み
   （`timer_us_gettime64`、`thd_sleep`、`dcload_type`、`KOS_VERSION_STRING`）。G1 バスの排他は `g1_ata_mutex_lock/unlock` ではなく
   `_g1_ata_sem` を `sem_wait` / `sem_signal` で直接使う（`g1_ata_mutex_unlock` は DEVICE_SELECT に書き込んでしまい、
   純正のもう一方のデバイスに触れうるため）。リンクで `_g1_ata_sem` が未定義になったら報告する。
   - **実機（2026-11-21〜22）**：DreamSDK（KOS 2.3.0、gcc 13.2.0）でビルドが通り、`_g1_ata_sem` のリンクエラーは出なかった（R117）。
2. **PACKET のシーケンス**：openMenu の `gdemu_sdk.c` と同じ手順（BSY/DRQ 待ち → 0xA0 → DRQ → 6 ワード → 割り込み → ステータス）に、
   タイムアウトと「実行前に STATUS を読んで古い割り込みを消す」を足した。割り込みの待ちは `EXT_INT_STAT` の bit 0。
   **最初の `TUR_W0` が `rc=timeout` なら、この待ちを疑う**（ALTSTATUS の BSY 待ちだけに変える案がある）。
   - **実機（2026-11-21〜22）**：PACKET のシーケンスは動いた。最初の TUR は、G1 バスの保護を開けたあとで `rc=ok`（`us=17`）。1 回目の `rc=timeout`（STATUS・IRQ が `0xae`）の原因は割り込みの待ちではなく、バス保護が閉じていたこと（R117、項目 7 も参照）。
3. **PIO のバイト数制限**：PACKET の前に LBA Mid/High = 0x0800 を書く（SPI 仕様どおり。BIOS も書く）。openMenu は書かないが、これは**意図的な違い**。純正が無視するか、1 セクタずつ DRQ を返すかは未確認。
   データ読み出しのループはどちらでも動く作りだが、`rc=proto` が出たらここ。
   - **実機（2026-11-21〜22）**：11 件の記録のどれにも `rc=proto` は無く、IP_READ（7 セクタ、`bytes=14336`）や 16 セクタずつのスループット読みも `rc=ok`（R117・R119）。純正が LBA Mid/High を無視したのか、1 セクタずつ DRQ を返したのかは、記録からは不明。
4. **CD_READ の CDB**：バイト1 = `0x24` は SPI 仕様で確認済み（bit 5 = Data、bit 3〜1 = 期待セクタ種別 `010` = Mode 1、bit 0 = FAD 指定）。
   `0x20` = データ・種別を問わない、`0x28` = Mode 2 Form 1（CDI / MIL-CD）。`switch=off` のときの dcload-ip の CDI は Mode 2 Form 1 なので、
   種別を検査するドライブでは `0x24` が `rc=check` を返しうる。その実行では `cdread_flags=0x20` を指定する。`cdread_flags` で変えられる。
   `IP_READ` が `rc=check` なら別の値を試す（`CDREAD_FLAGS` 行を見れば、4 つのフラグ値のうちどれが通るかが 1 回の実行で分かる）。返るバイト数（`bytes=`）が 2048 × 件数か、2352 × 件数かは、**記録を見て決まる**
   （`HASH` の `ssz` に反映される）。
   - **実機（2026-11-21〜22）**：純正は期待セクタ種別を検査する。GDI（Mode 1）は `0x24` が通り `0x28` が CHECK（5/64/00）、CDI（Mode 2 Form 1）はその逆。`0x20` は両方通り、`0x10` は 2352 バイトを返す。そのため種別が混ざるイメージでは **`cdread_flags=0x20` が安全**（R119・R120）。`0x20` / `0x28` の CRC は同じ。
5. **REQ_MODE の引数**：NetGD の `ide.rs` は (offset 18, len 8) と (0, 10) だけ受け付ける。純正が他の組を返すかを見るため `(0, 32)` も送る。
   - **実機（2026-11-21〜22）**：`(0, 32)` は 32 バイトを返した。末尾は `...SE      Rev 6.42990316`（本物の GD-ROM ドライブと同じ文字列）。(0, 10)・(18, 8) も `rc=ok`（R117）。
6. **IP.BIN の FAD 自動判定**：REQ_SES(2) の返事から取る。使える値が無ければ、GET_TOC の最後の有効な領域の最初のデータトラックの開始 FAD、それも無ければ 45150（tools/mksdcard の CDI 04/05 など GD-ROM 以外の像向け）。純正の GD-ROM では 45150 のはずだが、D-4 / D-1 の調査対象そのものなので、
   食い違うときは `ip_fad=45150` を指定して再実行する。
   - **実機（2026-11-21〜22）**：CDI では REQ_SES(2) が使え（dcload-ip の CDI で FAD 11852、`IP_READ fad=11852`）、GD-ROM（03）では REQ_SES(2) が CHECK（純正の GD-ROM のセッションは 1 つ。R119）なので TOC からの判定になり、`IP_READ fad=45150` で `rc=ok`。`ip_fad` の指定は要らなかった。
7. **KOS の cdrom ドライバ**：`main.c` は `KOS_INIT_FLAGS(INIT_DEFAULT & ~INIT_CDROM)` で、`cdrom_init` も `fs_iso9660` の vblank ポーリングも走らせない
   （純正ドライブに KOS が勝手にコマンドを送らない）。raw アクセスは `_g1_ata_sem` で直列化している。切替後に BIOS 経由の `/cd` を使うことはしていない。
   **実機で判明（LOG R117）**：`cdrom_init` を走らせないと G1 バスの保護が閉じたままで、ドライブのレジスタがすべて同じ値（`0xae`）に読める。
   そのため `gdraw_bus_prepare` が、プローブの前に `cdrom_init` と同じバスの確認（0x005F74E4 と BIOS の読み出し）だけを行う。結果は記録のヘッダ `# g1bus ...` に残る。
8. **時計**：Dreamcast の RTC が正しければファイル名に日時が入る。無ければ `seq<番号>`。
   - **実機（2026-11-21〜22）**：Dreamcast の RTC は未設定で、記録は `seq00000`〜`seq00010` のファイル名になった（日時は入らない）。
9. **`/pc/` の書き込み**：4 KB 単位 + `fflush`（ホストテストで確認）。実機で固まったら `PCOUT_CHUNK` を 2048 に下げる。
   - **実機（2026-11-21〜22）**：約 147 KB までの記録（最大 149,566 バイト）を 4 KB 単位で `/pc/` に書けた。固まらなかった（`PCOUT_CHUNK` は変更不要）。

### 初回の手順（チェックリスト）

1. ビルドが通る → エラーは全文を渡す。 **［済：R117］**
2. **まず NetGD でも純正でもなく、`switch=off` で、いま入っているデバイスに対して**実行する（`label=` を正しく）。 **［済：R117。1 回目は G1 バス保護で ABORT、`gdraw_bus_prepare` で修正した 2 回目が成功］**
3. コンソールで `[gdprobe] record: ...` が出て、`TUR_W*` の行が出ること。`rc=timeout` / `busy` が並んで `ABORT` になったら、上の 2 を疑う（記録は `/pc/` にそこまで残る）。 **［済：R117］**
4. 記録ファイルを開き、`GET_TOC_1` の `bytes=408`、`REQ_SES_2` の `data=` の FAD（45150 = `00b05e`）、`IP_READ` の `rc=ok` を確認。 **［済：R117。`GET_TOC_1` の `bytes=408`、`IP_READ` は `rc=ok`］**
5. `python compare.py --image <GDI> --img 0 rec.txt` で `hashes: N match, 0 mismatch`。 **［済：R119・R120。`--manifest` で HASH 1,050 / 1,056 / 2,098 件が全一致、不一致 0］**
6. ここまで良ければ `switch=on` と `images=` を有効にし、**最初は `images=3` だけ**で、`SW_RESULT` と READY までの時間を確認してから全フォルダ（`3,4,5,6,7`、`return_to=2`）に広げる。 **［済：R119（`images=3` → `3,4,5,6,7`）。READY までは約 0.33〜0.35 秒（`open_time`・`detect_time` が 150 / 150 のとき。R129・R135 で INI 依存と判明）］**
7. 純正での記録を、`docs/research/probe-records/` に置く（チャットに渡してもよい）。 **［済：`stock_20261121_01`〜`stock_20261122_11`］**
