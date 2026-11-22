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
  | （レジスタの受動読み出し） | — | コマンドではない。ALTSTATUS（0xA05F7018、CDB フェーズの `cdbst`）と GD-ROM ステータス `SECNR`（0xA05F708C、`secnr`）を**読むだけ**。`swap_regpoll` では ERROR・割り込み理由も、`swap_devreset` のリセット直後はバイト数レジスタ（0xA05F7090/94）も読む。STATUS は読まない。書くレジスタは FEATURES・バイト数・COMMAND・DATA のまま増えない |
  | GDEMU バージョン | `52 00` | 8 バイトのバージョンを読む |
  | **GDEMU 切替（オプトイン）** | `52 82 n` | フォルダ n のイメージをマウントする。openMenu がゲームを起動するときと同じ。設定で `switch=on` のときだけ許可 |
  | IDENTIFY PACKET DEVICE（オプトイン） | ATA 0xA1 | 機器の識別文字列（GD-ROM は 80 バイトを返す）。設定で `identify=on` のときだけ |
  | **DEVICE RESET（オプトイン、R148）** | ATA 0x08 | パケットデバイスのインターフェースの状態を初期化する（ATA-4 8.7）。データも媒体への書き込みも無い。設定で `switch=on` と `swap_devreset=on` のときだけ、切替の窓の中で 1 回ずつ。COMMAND レジスタへの書き込みで、DEVICE_SELECT には書かない |

  ATA のコマンド（PACKET 以外）も 1 つの表（`whitelist.c` の `wl_ata_table`）にあるものだけ。0xA1 と 0x08 以外の 254 個は、どの設定でも通らない（ホストテストで全数確認）。

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
| `swap_regpoll` | `off` | **R148**。`on` で、`images` の各切替（`52 82 n`）のあと `swap_regpoll_ms` のあいだ**コマンドを送らず**レジスタだけを読み、変化を `SW_REGPOLL` に記録。そのあと通常の TUR / REQ_ERROR の流れ（`return_to` の切替には付けない）。`switch=on` のときだけ |
| `swap_regpoll_ms` | 1000 | コマンドなしで観測する長さ（100〜10000）。`switch_timeout_ms` はこのあとから数える |
| `swap_regpoll_interval_ms` | 2 | 読む間隔（1〜50 ms）。CPU のタイマーで待つ（バスには触れない） |
| `swap_devreset` | `off` | **R148**。`on` で、各イメージの試験一式のあと、同じイメージへ `swap_devreset_ms` の値ごとにもう一度切り替え、`52 82 n` からその ms 後に ATA DEVICE RESET を送る（行の名前に `R<ms>_` が付く）。リセットの前後もレジスタを観測する。`switch=on` のときだけ |
| `swap_devreset_ms` | `50,200` | リセットの時刻（最大 4 個、0〜5000 ms）。既定は open 段階（50）と detect 段階（200）。観測は少なくとも「時刻 + 300 ms」まで続ける。1 つでも不正な値があれば行全体を無視（数える） |
| `swap_reswitch` | `off` | **seq 17**。`on` で、各イメージの試験一式のあと、**いまマウントされている同じフォルダ**へもう一度 `52 82 n` を送り、`swap_reswitch_wait_ms` のあいだコマンドを送らずレジスタだけを読み（`RS_SW_REGPOLL`）、**DEVICE RESET は送らず**、そのあと通常の TUR / REQ_ERROR の流れ（`RS_SW_POLL`・`RS_SW_RESULT`・`RS_REQ_STAT`・`RS_REQ_ERROR`）。UNIT ATTENTION 6/28/00 が来たかは `RS_SW_RESULT` の `ua_28=0/1`。`switch=on` のときだけ |
| `swap_reswitch_wait_ms` | 1000 | 上の観測時間（100〜10000）。`switch_timeout_ms` はこのあとから数える |
| `devreset_only` | `off` | **seq 17**。`on` で、各イメージの試験一式のあと（`swap_reswitch` があればその後）、**切替コマンドを 1 つも送らず**、まず 500 ms 何も送らずに待ち（`DR_IDLE idle_ms=500`）、ATA DEVICE RESET を送ってシグネチャのレジスタを `DR_SW_DEVRESET`（`delay_ms=0`）に記録、`devreset_only_wait_ms` のあいだレジスタを読み（`DR_SW_REGPOLL`）、TUR / REQ_ERROR の流れ（`DR_SW_POLL`・`DR_SW_RESULT` の `ua_28`・`DR_REQ_STAT`・`DR_REQ_ERROR`）。DEVICE RESET の許可は `swap_devreset` と同じ許可リスト（`switch=on` かつ `swap_devreset=on` または `devreset_only=on`）。`switch=on` のときだけ |
| `devreset_only_wait_ms` | 1000 | リセット後の観測時間（100〜10000） |
| `devreset_cmd` | `off` | **seq 20**。`on` で、各イメージの試験一式のあと、`devreset_cmd_ms` の値ごとに：500 ms 何も送らずに待ち（`DC<ms>_IDLE`）、ATA DEVICE RESET（`DC<ms>_SW_DEVRESET`）、`<ms>` ミリ秒待ってから **TEST UNIT READY**（`DC<ms>_CMD_TUR`：`rc`・`status`・`err`・`sense`・`secnr`・`altst`）、**REQ_STAT**（`DC<ms>_CMD_REQ_STAT`）、**REQ_ERROR**（`DC<ms>_CMD_REQ_ERROR`、保留中のセンス）、そのあと TUR を 10 ms ごとに READY になるまで（最大 2 秒）送り、変化だけを `DC<ms>_CMD_POLL` に、READY までの時間（リセットコマンドから）を `DC<ms>_CMD_RESULT` の `ready_ms` に書く。**リセット中の再検出のあいだ純正がどう答えるか**（NetGD は NOT READY 2/04/01）を見る。DEVICE RESET の許可は他と同じ許可リスト（`switch=on` かつ `swap_devreset` / `devreset_only` / `devreset_cmd` のどれか）。3 回続けてタイムアウトしたら中止。`switch=on` のときだけ |
| `devreset_cmd_ms` | `10,50,100` | **seq 20**。`devreset_cmd` の、DEVICE RESET から最初の TUR までの待ち（ms）。最大 4 個、0〜5000 |
| `head_probe` | `off` | **seq 21**。`on` で、各イメージの試験一式のあと（`switch=off` ならマウント中のイメージで）ヘッド位置の試験（下記「seq 21 の実行計画」、`HD_*` 行） |
| `head_flags` | `0x20` | **seq 21**。普通の読み出しケースの CD_READ バイト1（bit 0 は 0 に強制。許可リストを通らない値は `rc=refused`） |
| `head_counts` | `1,2,7,16` | **seq 21**。普通の読み出しケースのセクタ数（最大 6 個、1〜16） |
| `head_late_ms` | 300 | **seq 21**。各ケースの 2 回目の REQ_STAT までの待ち（0 = 送らない、0〜5000） |
| `head_idle_ms` | 2000 | **seq 21**。最後の `IDLE` ケースの待ち（0 = 省略、0〜10000） |
| `head_scd` | `on` | **seq 21**。各ケースで GET_SCD 形式 1 も読む |
| `gap_probe` | `off` | **seq 22**。`on` で、各イメージの試験一式（と `head_probe`）のあと、TOC の端の読み出し試験（下記「seq 22 の実行計画」、`GP_*` 行） |
| `gap_flags` | `0x20` | **seq 22**。この試験の CD_READ バイト1（bit 0 は 0 に強制。許可リストを通らない値は `rc=refused`） |
| `gap_late_ms` | 300 | **seq 22**。各ケースの 2 回目の REQ_STAT までの待ち（0 = 送らない、0〜5000） |
| `gap_scd` | `on` | **seq 22**。各ケースで GET_SCD 形式 1 も読む |
| `gap_track_end` | 0 | **seq 22**。トラック 1 のあとの隙間の最初の FAD（0 = 1 セクタ読みの二分探索で探す、`GP_SCAN_R` 行） |
| `gap_leadout` | `0,1,100` | **seq 22**。リードアウトからのずれ（最大 4 個、0〜10000） |
| `gap_low` | `0,100` | **seq 22**。150 未満の FAD（最大 4 個、0〜149） |
| `gap_count0` | `on` | **seq 22**。件数 0 の CD_READ。許可リストは 1〜64 なので送らずに `GP_CNT0_SKIP` を書く |
| `gap_crc` | `off` | **seq 23**。`on` で `GP_*` の読み出し行すべてに `crc=`（返ったバイトの CRC-32、無ければ `-`）、複数セクタなら `scrc=`（セクタごと）、バッファに入りきらなければ `stored=` |
| `pregap_probe` | `off` | **seq 23**。`on` でプリギャップとまたがる読み出しの試験（下記「seq 23 の実行計画」）。`gap_probe=off` でも `GP_START` から実行 |
| `pregap_low` | `0,1,100,149` | **seq 23**。トラック 1 のプリギャップで読む FAD（最大 4 個、0〜149） |
| `pregap_offs` | `-156,-151,-150,-149,-100,-2,-1,0` | **seq 23**。2 番目以降の各トラックの開始からのずれ（最大 8 個、−300〜300） |
| `pregap_cross` | 16 | **seq 23**。またがる読み出しのセクタ数（2〜16。許可リストは 1〜64、バッファは 2352 B × 16 まで） |
| `risky_cross` | `inline` | **seq 24**。データの上から始まりその終わりを越える読み出し（`XEND`・`XLD`・`XLO`、データ → プリギャップの `XGAP<t>`）をどこで送るか。`inline` = seq 23 と同じ各イメージの中、`end` = すべてのイメージの通常の試験のあと、イメージを順にもう一度マウントして（各読み出しの前に `GP_RISKY` 行を書いて送り出す）、`off` = 送らない（`GP_RISKY_SKIP`） |
| `ctl_probe` | `off` | **seq 24**。`on` で、トラックの外で報告される CTRL の試験（`CTLF_*`・`CTLR_*`・`GP_CTL`。下記「seq 24 の実行計画」） |

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
- **`swap_regpoll=on`**（R148）：2 の `52 82 n` のすぐあと、`swap_regpoll_ms` のあいだ**パケットコマンドを 1 つも送らず**、
  ALTSTATUS・ERROR・割り込み理由・SECNR を `swap_regpoll_interval_ms` ごとに読み、どれかが変わるたびに `SW_REGPOLL` を 1 行書く。
  終わりに `SW_REGPOLL_END`、そのあと従来の TUR / REQ_ERROR の流れ（`SW_POLL`、UNIT ATTENTION はここで消える）。
- **`swap_devreset=on`**（R148）：2 の試験一式のあと、同じ n に `swap_devreset_ms` の値ごとに切り替え直す（`R50_SW_CMD` など）。
  `52 82 n` から指定の ms で DEVICE RESET を送り、BSY が下がるのを待って（上限 `timeout_ms`、最低 2 ms は見る）レジスタを `R50_SW_DEVRESET` に記録、
  観測を続け（`R50_SW_REGPOLL`）、TUR / REQ_ERROR で READY を待ち（`R50_SW_POLL`・`R50_SW_RESULT`）、最後に `R50_REQ_STAT`・`R50_REQ_ERROR` を 1 回ずつ。
  切替コマンドが `rc=ok` でなければ、リセットも観測もしない。
- **`swap_reswitch=on`**（seq 17）：Record 15・16 は「同じフォルダへ切り替え直し → DEVICE RESET」だったので、どちらの効果か分からない。
  これは**切り替え直しだけ**：2 の試験一式のあと、マウント済みの同じ n に `52 82 n`（`RS_SW_CMD`）、`swap_reswitch_wait_ms` のあいだコマンドなしでレジスタ観測
  （`RS_SW_REGPOLL`・`RS_SW_REGPOLL_END`）、TUR / REQ_ERROR（`RS_SW_POLL`）で UNIT ATTENTION（6/28/00）が来るかを見る。切替コマンドが `rc=ok` でなければ観測もしない。
- **`devreset_only=on`**（seq 17）：逆に**DEVICE RESET だけ**：切替コマンドなしで 500 ms 待ち（`DR_IDLE`）、DEVICE RESET（`DR_SW_DEVRESET`）、観測、TUR / REQ_ERROR。
  `swap_reswitch` と `devreset_only` は独立（片方だけでも両方でもよい）。両方 `on` のときの順序は、イメージの試験一式 → `RS_*` → `DR_*`。
  どちらも 3 回続けてタイムアウトしたら中止（`ABORT`）。読み出しだけ・時間制限つきで、新しく送るコマンドは無い（`52 82` と、リセットは既存の許可リスト）。
- **データトラックの無いイメージ**（音声だけの CDI など）：TOC に有効なトラックがあってデータトラックが 1 つも無ければ、`NO_DATA_TRACK audio_fad=..` を書いて
  `IP_READ`・スループット・シークを飛ばし、`CDREAD_FLAGS` だけを最初の音声トラックで行う（REQ_SES(2) が値を返しても TOC を優先。存在しない IP.BIN を読みに行かない）。
  `ip_fad=` を指定すれば従来どおり読む（応答はそのまま記録。どの待ちにもタイムアウトがある）。

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
- **受動的な観測（R137 a・b）**：PACKET 経由の行には、`bytes=` の次に次の 2 つが付く。ハードウェアの転送層（`gdraw.c`）だけが付ける（古い記録・ホストテストの偽ドライブには無い）。
  - `cdbst=0x58`：CDB フェーズ（PACKET の後に DRQ が立った瞬間）の ALTSTATUS。DRQ を見つけた読み出しそのものの値で、追加のバスアクセスは無い。
    直前の CHECK の後でも純正が 0x58（DRDY あり）か 0x18（DRDY なし）かが分かる（NetGD は CHECK を 0x11 にするので 0x18 に見える疑い、R137 a）。
  - `secnr=0x82`：GD-ROM の「セクタ番号」ステータスレジスタ（0xA05F708C）をコマンド完了時に読んだ値。下位ニブル = ドライブの状態、上位ニブル = ディスク形式。
    KOS `g1ata.c:90` の `G1_ATA_LBA_LOW`（= `G1_ATA_CHS_SECTOR`、Read/Write）と同じアドレスだが、**読むだけ**で書かない（読んでも割り込みは消えない）。
    `SW_POLL` 行にも付き、**`secnr` だけが変わった場合も状態変化として 1 行書く**（R137 b）。
  - `compare.py` は `cdbst` / `secnr` を**両方の記録にあるときだけ**比べる（古い記録との比較は差分にならない）。
- `HASH` = 読んだセクタごとの CRC-32（`ssz` = 1 セクタのバイト数。2048 = ユーザーデータ、2352 = 生セクタ）。
- **R148 の行**（`swap_regpoll` / `swap_devreset`、データトラックの無いイメージ）：

  ```
  img=3 probe=SW_REGPOLL t_ms=1 sample=1 altst=0x50 err=0x00 irr=0x03 secnr=0x06
  img=3 probe=SW_REGPOLL t_ms=151 sample=76 altst=0x50 err=0x00 irr=0x03 secnr=0x00
  img=3 probe=SW_REGPOLL_END t_ms=1001 samples=500 changes=3 stop=time
  img=3 probe=R50_SW_DEVRESET delay_ms=50 t_ms=51 rc=ok reset_us=2000 bsy_seen=0 altst=0x50 err=0x01 irr=0x01 secnr=0x06 bcl=0x14 bch=0xeb
  img=11 probe=NO_DATA_TRACK audio_fad=150 reads=skipped
  ```

  - `t_ms` = `52 82 n` を送った時刻からのミリ秒、`sample` = 何回目の読み出しか。値（`altst`・`err`・`irr`・`secnr`）が変わったときだけ書く（リセットの直後は必ず 1 行）。
  - `SW_REGPOLL_END` の `stop` = `time`（予定どおり）／`iteration-cap`（時計が進まないときの上限）／`bus-lock`（G1 の排他が取れない）／`abort`／`no-register-access`（古い転送層）。
  - `SW_DEVRESET`：`delay_ms` = 設定の時刻、`t_ms` = 実際に送った時刻、`rc` = `ok`／`timeout`（BSY が下がらない）／`refused`（許可リスト）、
    `reset_us` = 書き込みから BSY が下がるまで、`bsy_seen` = BSY を一度でも見たか、`bcl`/`bch` = バイト数レジスタ（ATAPI のシグネチャは 0x14/0xEB）。
  - `compare.py` は SW_POLL と同じく出現順で対応づけ、`t_ms`・`sample`・`samples`・`reset_us` は時間として扱う（差分に数えない。比が大きければ「TIMING」の注記）。

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
| `Makefile` / `build.ps1` / `profiles/gdprobe.ps1` / `gdprobe.cfg.example` / `gdprobe.cfg.r137.example`（R137 の実行用）/ `gdprobe.cfg.r148.example`（R148 の実行用）/ `gdprobe.cfg.seq17.example`・`gdprobe.cfg.seq18.example`・`gdprobe.cfg.seq20.example`（seq 17・18・20 の実行用） | ビルド、dc_auto のプリセット、設定の雛形 | — |
| `tests/` | `sh tests/run_tests.sh`（gcc と python が要る。make は不要）。DreamSDK の `python` は環境変数を渡さないので、C の偽ドライブの記録を使う 2 件はスキップになる（Windows の `py tests/test_compare.py` なら通る） | — |

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

## R137 の実行計画（a・b・g を 1 回の純正実行で測る）

**状態：用意しただけ。実機では未実行。** 結果は実機の記録を見てから書く（LOG R137 の項目 a・b・g）。

準備：

1. `python tools/mksdcard/mksdcard.py E:\variants_out --set variants`（`08`・`09`・`10`。`10` は約 10.8 MB。`--small` なら約 4 MB）を作り、
   `python tools/mksdcard/verify.py E:\variants_out` で表が一致し「Mode 1 sectors」と出ることを確かめる。
2. SD カードの既存の `01`〜`07` の横に **`10` だけ**をコピーする（`08`・`09` は今回は不要）。ファイルサイズを確認する（途中で切れたイメージは結果を壊す）。
3. `gdprobe.cfg.r137.example` を `host\gdprobe.cfg` にコピーして `label=` を確認する。`switch=on`、`images=4,10,3,7`、`return_to=2`、`cdread_flags_test=on`。
   **まず `switch=off` で 1 回**走らせる流れは今までどおり（安全のルール参照）。今回の追加はレジスタの読み出しだけなので、書くレジスタは増えない。
4. 実行は 1 回ずつ（`dc_auto.ps1 -Preset ...\profiles\gdprobe.ps1`）。記録は `host\gdprobe_stock_*.txt`。

見るところ：

| 項目 | 記録のどこを見るか | 読み方 |
|---|---|---|
| **a** CDB フェーズの status | 各行の `cdbst=`。とくに `TUR_W0`（起動直後の CHECK の後）と、`rc=check` の行の**次の行**の `cdbst` | 純正が 0x58 なら DRDY が立っている（NetGD は 0x18 に見える疑い）。0x18 なら純正も同じ。全行が同じ値か、直前の結果で変わるかも見る |
| **b** SECNR | `SW_POLL` 行の `secnr=`（`img=4`・`10`・`3`・`7` の切替中。変化のたびに 1 行）と、各プローブ行の `secnr=` | 切替中の値の並び（OPEN 6 / BUSY 0 / NODISC 7 などの下位ニブル、上位ニブル = ディスク形式）と、空フォルダ `07` での値。READY になったときの値。R126 の推測との違いを見る |
| **g** Mode 1 の CDI | `img=10` の `CDREAD_FLAGS` 行（`flags=0x20` / `0x24` / `0x28` / `0x10`）を `img=4`（Mode 2 Form 1 の CDI）・`img=3`（GDI）と並べる | `0x24` が `rc=ok`・`0x28` が `rc=check`（`sense=5/64/00`）なら、純正は CDI かどうかでなく**トラックのモード**で検査している（NetGD の `is_cdi` 判定は誤り）。`0x24` も `0x28` も通る／`0x24` が `check` で `0x28` が通るなら、CDI なら Mode 2 として扱う。`ssz` と `crc`（`compare.py --manifest <variants_out>\NETGD-PROBE-MANIFEST.json 記録` で照合）も見る |

記録の照合：`python compare.py --manifest E:\variants_out\NETGD-PROBE-MANIFEST.json host\gdprobe_stock_....txt`（`img=10` の `HASH` と `CDREAD_FLAGS` が再計算で照合される。
03〜06 を同じ記録で照合するときは、通常カードのマニフェストを別に渡す）。

## R148 の実行計画（切替の窓をコマンドなしで観測・窓の中の DEVICE RESET・音声だけの CDI）

**状態：用意しただけ。実機では未実行。** NetGD の R148（`feat/stock-rules`）が推測で書いた 3 点を、1 回の純正実行で確かめる。

| 問い | NetGD（R148）の推測 | 記録のどこを見るか |
|---|---|---|
| 1. コマンドを送らなくても、純正の SECNR は時間だけで `0x06` → `0x00` → `0xX1`（空フォルダは `0x07`）と進むか | 時間で進める（`ide_task` のタイマー） | `SW_REGPOLL` 行の `t_ms` と `secnr`（`img=4`・`3`・`11`・`7`）。最後の `SW_REGPOLL` の値が READY の値か、`SW_POLL` の最初の行（`t_ms` ≥ 1000）が 2/04/01・6/28/00 のどちらか |
| 2. 窓の中の DEVICE RESET の直後に純正が示す SECNR（open 段階 50 ms・detect 段階 200 ms） | 窓の中のリセットは窓の値（`window_secnr`）を書く | `R50_SW_DEVRESET`・`R200_SW_DEVRESET` 行の `secnr`・`altst`・`err`・`irr`・`bcl`/`bch`（シグネチャ 0x14/0xEB が出るか）、そのあとの `R50_SW_REGPOLL` の並び、`R50_SW_POLL` のセンス（6/29/00 が出るか）、`R50_REQ_STAT` |
| 3. 音声だけの CDI のディスク形式 | 0x00（Flycast と同じ） | `img=11` の `REQ_STAT` の `data=` のバイト 1 の上位ニブル、各行の `secnr=` の上位ニブル、`SW_REGPOLL` の READY の値、`NO_DATA_TRACK` と `CDREAD_FLAGS`（音声 2,352 B） |

準備：

1. `python tools/mksdcard/mksdcard.py OUT --set variants --only 11` で `11`（約 2.4 MB、音声 2 トラックだけ、セッション 1 つ）を作り、
   `python tools/mksdcard/verify.py OUT` で「audio only (no data track)」と出ることを確かめる。SD カードの既存の `01`〜`07`・`10` の横に **`11` だけ**をコピーし、MD5 を照合する。
2. `gdprobe.cfg.r148.example` を `host\gdprobe.cfg` にコピーする（`label=stock`、`seq=15`、`images=4,3,11,7`、`return_to=2`、`swap_regpoll=on`、`swap_devreset=on`、`swap_devreset_ms=50,200`、`switch_timeout_ms=5000`、`cdread_flags=0x20`）。
3. 実行は 1 回（`dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1`）。所要は約 1〜2 分（空フォルダ 07 は 3 回とも 5 秒の上限まで待つ）。

注意（安全）：

- **新しく送るのは ATA DEVICE RESET（0x08）だけ**で、`swap_devreset=on` のときに限る（ATA の許可リスト `wl_ata_table`）。媒体にも SD にも何も書かない。
  純正が `52 82` でイメージを読み込んでいる最中のリセットなので、純正の挙動は分からない。固まったら（`rc=timeout` が続き `ABORT`）Dreamcast の電源を入れ直す。
  切替コマンド自体が失敗したとき（`SW_CMD` が `rc=ok` でない）は、リセットもレジスタの観測もしない。
- レジスタの観測（`swap_regpoll`）は読むだけ（ALTSTATUS・ERROR・割り込み理由・SECNR、リセットのあとはバイト数レジスタも）。STATUS は読まない（割り込みを消さない）。
- 純正が音声だけの CDI を受け付けない場合も、それ自体が結果（`img=11` の `SW_RESULT` と `REQ_ERROR`）。

照合：`python compare.py --manifest <OUT>\NETGD-PROBE-MANIFEST.json host\gdprobe_stock_seq00015.txt` で `img=11` の `CDREAD_FLAGS`（`0x20`・`0x10` の PCM）を照合する
（03〜05・10 は、それぞれのカードのマニフェストを別に渡す）。

## seq 17 の実行計画（切り替え直しだけ／DEVICE RESET だけ、続けて detect_time = 400 の seq 18）

**状態：用意しただけ。実機では未実行。** Record 15・16 は常に「同じフォルダへ切り替え直し → DEVICE RESET」だった。残る問い：

| 問い | 記録のどこを見るか |
|---|---|
| 1. 切り替え直しだけ（リセットなし）で UNIT ATTENTION（6/28/00）が来るか。SECNR は `0x06` → `0x00` → `0xX1` と進むか | `RS_SW_REGPOLL` の `secnr` の並びと `t_ms`、`RS_SW_POLL` の `sense=`、`RS_SW_RESULT` の `ua_28` |
| 2. 切り替えなしの DEVICE RESET だけで UNIT ATTENTION が来るか（6/29/00 か、6/28/00 か、来ないか）。SECNR は動くか | `DR_SW_DEVRESET`（シグネチャ・`secnr`）、`DR_SW_REGPOLL`、`DR_SW_POLL` の `sense=`、`DR_SW_RESULT` の `ua_28` |
| 3.（seq 18）リセット後の約 150 ms は `detect_time` か | seq 18 の `DR_SW_REGPOLL` の変化時刻を seq 17 と比べる（150 ms のまま → `open_time`、400 ms 近く → `detect_time`） |

準備と実行：

1. `gdprobe.cfg.seq17.example` を `host\gdprobe.cfg` にコピー（`label=stock`、`seq=17`、`images=4,3,7`、`return_to=2`、`switch_timeout_ms=5000`、`swap_regpoll=on`、
   `swap_devreset=off`、`swap_reswitch=on`、`devreset_only=on`、`cdread_flags_test=off`、`throughput=off`、`seek=off`、`identify=off`）。SD カードは R148 までと同じ（`02`・`03`・`04`・`07`）。
2. `cd H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1` を 1 回。
   所要は約 1 分（各イメージ：切替 1 秒 + 再切替 1 秒 + 待ち 0.5 秒 + リセット 1 秒 + TUR。空フォルダ 07 は各 5 秒の上限まで待つ）。
3. 次に、SD カードの `GDEMU.ini` で `detect_time = 400`（`open_time` は 150 のまま）にして、`gdprobe.cfg.seq18.example`（`seq=18`、他は同じ）で同じ手順をもう 1 回。終わったら `detect_time` を元に戻す。

安全：新しく送るコマンドは無い（`52 82 n` と ATA DEVICE RESET は R148 までと同じ許可リスト）。すべて読み取り専用・時間制限つき、3 回続けてタイムアウトしたら中止。
固まったら Dreamcast の電源を入れ直す。照合は `compare.py` で seq 17 と seq 18（または NetGD の記録）を比べる（`t_ms`・`idle_ms` などの時間は差分にならず、`ua_28` と `sense` は差分になる）。

## seq 20 の実行計画（DEVICE RESET の再検出中、純正はコマンドにどう答えるか）

**状態：用意しただけ。実機では未実行。** NetGD は DEVICE RESET のあと（`detect_time` のあいだ）に来たコマンドへ NOT READY 2/04/01 を返す。
KOS の `cdrom_reinit_ex` はセンスキー 2 を「ディスクなし」と扱うかもしれず、起動に関わる。純正がこの時間帯に何を返すかを記録する。

| 問い | 記録のどこを見るか |
|---|---|
| 1. リセットの 10・50・100 ms 後に TUR を送ると、純正は何を返すか（CHECK か OK か、センスは 2/04/01 か 6/29/00 か） | `DC<ms>_CMD_TUR` の `rc`・`status`・`err`・`sense`・`secnr`・`altst` |
| 2. 同じ時刻の REQ_STAT・保留中のセンス | `DC<ms>_CMD_REQ_STAT`（`data`）、`DC<ms>_CMD_REQ_ERROR`（`data` の 2 バイト目の下位 4 ビット = センスキー、8・9 バイト目 = ASC・ASCQ） |
| 3. READY になるまでの状態の移り変わりと、リセットから READY までの時間 | `DC<ms>_CMD_POLL`（変化ごと）、`DC<ms>_CMD_RESULT` の `ready_ms`（`t_ms`・`polls`・`changes` も） |

準備と実行：

1. `gdprobe.cfg.seq20.example` を `host\gdprobe.cfg` にコピー（`label=stock`、`seq=20`、`images=4,3,7`、`return_to=2`、`switch_timeout_ms=5000`、`swap_regpoll=off`、`swap_devreset=off`、
   `swap_reswitch=off`、`devreset_only=off`、`devreset_cmd=on`、`devreset_cmd_ms=10,50,100`、`cdread_flags_test=off`、`throughput=off`、`seek=off`、`identify=off`）。SD カードは seq 17 と同じ（`02`・`03`・`04`・`07`）。
2. `cd H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1` を 1 回。
   所要は約 1 分半（各イメージ：切替と試験一式 + 3 回 ×（待ち 0.5 秒 + リセット + コマンド + READY まで最大 2 秒）。空フォルダ 07 は READY にならないので各回 2 秒の上限まで）。

安全：新しく送るコマンドは無い（TUR・REQ_STAT・REQ_ERROR・`52 82 n`・ATA DEVICE RESET は既存の許可リスト）。すべて読み取り専用・時間制限つき、3 回続けてタイムアウトしたら中止。
固まったら Dreamcast の電源を入れ直す。照合は `compare.py`：`ready_ms`・`t_ms`・`polls` などの時間は差分にならず（比の注記だけ）、`rc`・`status`・`err`・`sense`・`secnr`・`altst`・`ready` は差分になる。

## seq 21 の実行計画（読み出しのあと、純正はヘッド位置をどう動かすか）

**状態：用意しただけ。実機では未実行。** Record 17 の再監査で、純正の REQ_STAT が報告するヘッド位置（バイト 3 = トラック、4 = インデックス、5〜7 = FAD）が
CD_READ のあとで動くことが分かった（NetGD は動かさない）。img=4 では読み出し前 `01 2f 01 01 01 00 00 96`（トラック 1・FAD 150）が、
あとで FAD 11852（1 セクタ読みの先頭。7 セクタの 0x24 IP_READ はそこで CHECK）、img=3 では 45157 = 45150 + 7（7 セクタ IP_READ の終わり。そのあと 45150 の 1 セクタ読みがあったのに）。
規則（最後の読み出しの先頭か終わりか、1 セクタと複数セクタの違い、種別 CHECK の読み出しで動くか）を 1 回の実行で読み取れるようにする。

`head_probe=on` のとき、各イメージの試験一式のあと（`switch=off` ならマウント中のイメージで）次のケースを順に実行する。D〜E は最後の TOC 領域の最初のデータトラック
（E = 次のトラックの開始、無ければリードアウト）、`len` = E − D。読み出しはすべて FAD 指定・16 セクタ以下・1 トラックの中。

| ケース | 送るもの |
|---|---|
| `BASE` | 何も送らない（位置の問い合わせだけ。試験一式の IP_READ のあとの位置） |
| `R<n>`（`head_counts` の各 n） | `head_flags`（既定 0x20）で n セクタを D + len × k / 16（k = 1, 2, …）から |
| `HIT` | 直前の `R<n>` の範囲の中の 1 セクタ（読み直し。バッファから返るかもしれない） |
| `MID7` / `END7` / `END1` / `START7` | トラックの中央から 7、E − 40 から 7、E − 2 から 1、D から 7 |
| `CHK24` / `CHK28` | 0x24 / 0x28 で 7 セクタ（D + len × 10/16、× 12/16）。Mode 1 なら 0x28、Mode 2 なら 0x24 が種別 CHECK になるはず。CHECK ならすぐ REQ_ERROR でセンスを読む |
| `TUR` / `TOC` / `SES` | TEST UNIT READY、GET_TOC 領域 0（408 バイト）、REQ_SES セッション 1 |
| `AUD1` / `AUD7` | 最初の音声トラック（長さ 32 以上のとき）の 1/4 から 1 セクタ、1/2 から 7 セクタ（0x20 なら 2352 B/セクタ、R144） |
| `IDLE` | `head_idle_ms`（既定 2000）何も送らずに待つ |

各ケースのあと：**REQ_STAT をすぐ**、**GET_SCD 形式 1**（`head_scd`）、**`head_late_ms`（既定 300）後にもう一度 REQ_STAT**。記録はケースごとに 2 行
（命令の行 `HD_<ケース>`、位置の行 `HD_<ケース>_POS`。`BASE`・`IDLE` は位置の行だけ）：

```
img=3 probe=HD_R7 flags=0x20 fad=139842 count=7 ssz=2048 sense=none cdb=... rc=ok ... bytes=14336 us=...
img=3 probe=HD_R7_POS stat_rc=ok st=pause trk=3 idx=1 pos=139849 d_start=+7 d_end=+0 moved=1 scd_rc=ok scd_trk=3 scd_idx=1 scd_pos=139849 late_rc=ok late_st=pause late_pos=139849 stat=018f... scd=0015000e...
```

- `st` = REQ_STAT バイト 0 の下位ニブル（busy / pause / standby / play / seek / scan / open / nodisc / retry / error）。`trk`・`idx`・`pos` = バイト 3・4・5〜7。
- `d_start` = pos − 読んだ先頭の FAD、`d_end` = pos − (先頭 + 件数)。**`d_end=+0` なら終わり、`d_start=+0` なら先頭、どちらでもなく `moved=0` なら動いていない。**
- `moved` = 直前の `_POS` 行の `pos` から変わったか（最初は `-`）。`TUR`・`TOC`・`SES`・`IDLE` で動くかはこれで見る。
- `scd_*` = GET_SCD 形式 1 のトラック・インデックス・絶対 FAD（バイト 5・6・11〜13）。`late_*` = 遅れて読んだ REQ_STAT（時間とともに先読みで進むか、pause → standby になるか）。
- `stat=`・`scd=` は生のバイト。問い合わせが失敗したら該当欄は `-`。`HD_START` にトラックの範囲（`data_fad`・`data_end`・`audio_fad`・`audio_end`）と、どのケース群を実行したか。

**読み方の例**：`R1_POS` が `moved=0` で `R2_POS` 以降が `d_end=+0` なら「複数セクタは終わりへ、1 セクタは動かさない」。`CHK28_POS`（GDI）・`CHK24_POS`（CDI Mode 2）が
`d_start=+0` なら「CHECK でもシークだけは行う」。`HIT_POS` と `R1_POS` の違いで、バッファから返した読み出しの扱いが分かる。

送らないもの：**CD_SEEK（0x21）・CD_PLAY（0x20）・CD_SCAN（0x22）は許可リストに意図的に無い**（`whitelist.c` のコメント）。ピックアップや音声の状態を動かす命令なので、今回も追加していない。
SEEK の効果は「読み出しで動いた位置」から推定する。**DMA の読み出しは無い**：`gdraw.c` は PIO だけ（FEATURES = 0）で、G1 DMA の転送路を持たない。PIO と DMA で位置の規則が違うかは、この試験では測れない。

準備と実行：

1. SD カードは `02`・`03`・`04`・`10`・`11`（R137・R148 で作った `10` と `11`）。
2. `gdprobe.cfg.seq21.example` を `host\gdprobe.cfg` にコピー（`label=stock`、`seq=21`、`images=4,3,10,11`、`return_to=2`、`switch_timeout_ms=5000`、他の切替・リセット試験は off、
   `cdread_flags=0x20`（試験一式の IP_READ を CDI でも CHECK にしない）、`cdread_flags_test=off`・`throughput=off`・`seek=off`、`head_probe=on`、`head_counts=1,2,7,16`、`head_late_ms=300`、`head_idle_ms=2000`、`head_scd=on`）。
3. `dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1` を 1 回。所要は約 1 分（各イメージ：16 ケース × 約 0.4 秒 + 待ち 2 秒）。

安全：新しく送るコマンドは無い（CD_READ・REQ_STAT・GET_SCD・REQ_ERROR・TUR・GET_TOC・REQ_SES・`52 82 n` は既存の許可リスト）。`head_flags` が許可リストに無い値なら送らずに `rc=refused` と記録する。
すべて読み取り専用・時間制限つき、3 回続けてタイムアウトしたら中止。照合は `compare.py`（`pos`・`moved`・`d_*`・`st`・`stat` などは差分、`us`・`idle_ms` は時間の注記だけ）。

記録のヘッダには `# config cdread_flags_test=.. seek_span=.. switch_timeout_ms=..` と `# config head_probe=.. head_flags=.. ...` の 2 行が増えた（それ以前の記録には無い。`compare.py` はヘッダを読まない）。

## seq 22 の実行計画（TOC の端の読み出しで、純正はヘッド位置をどう報告するか）

**状態：用意しただけ。実機では未実行。** 記録 21（LOG R172）の続き：隙間で REQ_STAT がトラック 2 を返したのは「次のトラック」か「直前の保持」か、
リードアウト・GD-ROM の領域の間・FAD 150 未満・件数 0 での動き。FAD はすべて実行時に各イメージの TOC（両領域）から計算し（`toc.c`）、行に書く。

`gap_probe=on` のとき、各イメージの試験一式（と `head_probe`）のあと、`GP_START`（TOC の要約と各ケース群の可否）と `GP_BASE_POS` に続いて：

| ケース | 送るもの |
|---|---|
| `TRK_*` (1) | トラック 1（A）と次のトラック（B）の間の隙間。`GP_SCAN_R`（1 セクタ読みで A の最後の読めるセクタを二分探索、約 15 回）→ `GP_SCAN`（`last_ok`・`gap_lo`・`gap_hi`・`gap_mid`）。そのあと `T1`（A+50）→ `GAPMID`、`T1END`（最後の読めるセクタ）→ `GAPLO`、`T2`（B+16）→ `GAPHI`（B−1）、`T2B`（B+32）→ `GAPMID2` |
| `LO_*` (2) | `LO_REF`（リードアウト − 2）、`LO_<o>`（リードアウト + o、`gap_leadout`） |
| `HDG_*` (3) | GD-ROM だけ：`GP_HDG`（範囲）、`HDG_REF`（LD リードアウト − 1）、`HDG_L0`（LD リードアウト）、`HDG_MID`、`HDG_HI`（45150 − 1）、`HDG_HD`（45150 + 16）、`HDG_MID2` |
| `LOW_*` (4) | `LOW_REF`（最初のデータトラック + 16）、`LOW_<f>`（`gap_low`） |
| `CNT0` (5) | 件数 0。許可リスト（1〜64）を変えていないので `GP_CNT0_SKIP ... reason=whitelist_count_1_64`（何も送らない） |

読み出しはすべて 1 セクタ・`gap_flags`。各ケースのあと seq 21 と同じ `GP_<ケース>_POS`（REQ_STAT、GET_SCD 形式 1、`gap_late_ms` 後の REQ_STAT）に、
`ctl`（REQ_STAT バイト 2 = CTRL/ADR）、`scd_ctl`（GET_SCD バイト 4）、`scd_rel`（バイト 7〜9 の相対位置）が増える。センスは CHECK のときすぐ REQ_ERROR で読む（`sense=5/21/00` など）。
その領域が無いイメージは `GP_TRK_SKIP`・`GP_LO_SKIP`・`GP_HDG_SKIP` に理由（`no_gap`・`one_track`・`no_hd_area` など）を書く。

準備と実行：`gdprobe.cfg.seq22.example` を `host\gdprobe.cfg` にコピー（`images=4,3,10`、11 はコメント）、`dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1` を 1 回。
所要は約 1 分（各イメージ：約 15 回の探索 + 約 20 ケース × 約 0.35 秒）。新しく送るコマンドは無い（既存の許可リストの CD_READ・REQ_STAT・GET_SCD・REQ_ERROR）。
記録のヘッダに `# config gap_probe=.. gap_flags=.. ... gap_leadout=.. gap_low=..` の行が増えた。

## seq 23 の実行計画（プリギャップの中身と、境界をまたぐ読み出し）

**状態：用意しただけ。実機では未実行。** 記録 22（LOG R174）で、純正の読める範囲は各トラックの [開始 − 150, データの終わり) と分かった。
seq 23 はプリギャップで純正が**何を返すか**（バイト数・CRC-32）と、境界をまたぐ読み出しの答え（センス・バイト数・CRC・ヘッド位置）を記録する。
`gdprobe.cfg.seq23.example` は `gap_probe=on`（seq 22 の各ケースも CRC つきでもう一度）・`gap_crc=on`・`pregap_probe=on`。FAD はすべて TOC から計算して行に書く。

| ケース | 送るもの |
|---|---|
| `GP_PG_START` | 両領域をまとめたトラック表（`tracks=番号/開始/CTRL`）、リードアウト、LD リードアウト |
| `PGL_<f>` | `pregap_low` の各 FAD を 1 セクタ（0・1・100・149） |
| `GP_PGB`、`PG<t>_M<o>` / `PG<t>_P<o>` | 2 番目以降の各トラック t の開始 + o（`pregap_offs`）を 1 セクタ。`GP_PGB` に選んだ FAD。GDI のトラック 2 なら 750・755・756・757・806・904・905・906、トラック 3 なら 44994・44999・45000・45001・45050・45148・45149・45150、CDI のトラック 2 なら 11696〜11852 |
| `XPRE<t>` | 開始 − 10 から `pregap_cross` セクタ（プリギャップ → 本体。トラック 1 は FAD 140、GDI のトラック 3 は 45140） |
| `XGAP<t>` | 開始 − 158 から（プリギャップの前 → プリギャップ。GDI のトラック 2 なら 748〜：トラック 1 のデータ → プリギャップ） |
| `GP_DSCAN_R` / `GP_DSCAN`、`XEND` | トラック 1 のデータの終わりを、トラック 2 のプリギャップより前（〜開始 − 151）で二分探索（`gap_track_end` があればそれ）。終わり − `pregap_cross`/2 から（データ → 隙間）。プリギャップまで続いていれば `GP_XEND_SKIP reason=no_gap` |
| `XLD` / `XLO` | GD-ROM の LD リードアウト − 8 から、リードアウト − 8 から（データ → 隙間・リードアウト） |

各読み出しのあと seq 22 と同じ `GP_<ケース>_POS`。`scrc=` のセクタごとの CRC は 1 セクタ読みの `crc=` と比べられる（同じセクタなら同じ値のはず）。

**件数 0 は今回も送らない（`GP_CNT0_SKIP reason=whitelist_count_1_64`）。** 許可リスト（`whitelist.c`）はこのプローブの安全の境界で、CD_READ は 1〜64 セクタだけを通す。
件数 0 を測るには許可リストを変える必要があり、それは**利用者の判断が要る**（エージェントは変えない）。

準備と実行：`gdprobe.cfg.seq23.example` を `host\gdprobe.cfg` にコピー（`images=4,3,10`）、`dc_auto.ps1 -Preset <このフォルダ>\profiles\gdprobe.ps1` を 1 回。
所要は約 1.5 分（各イメージ：seq 22 の約 20 ケース + seq 23 の約 20〜30 ケース × 約 0.35 秒 + 探索）。新しく送るコマンドは無い。
記録のヘッダに `# config gap_crc=.. pregap_probe=.. pregap_cross=.. pregap_low=.. pregap_offs=..` の行が増えた。

## seq 24 の実行計画（危ない読み出しを外して GDI・CDI 10 を先に、トラックの外の CTRL）

**状態：用意しただけ。実機では未実行。** 記録 23（LOG R176）では img=4 の `GP_XEND`（444 から 16 セクタ、データの終わりは 451）で純正が実在の 8 セクタを送ったあと BSY のまま止まり、
GDI 03・CDI 10 は記録できなかった。`gdprobe.cfg.seq24.example` は `images=3,10,4`（4 を最後）、seq 22・23 のケース（CRC つき）、`risky_cross=off`、`ctl_probe=on`。

**危ないと分類した読み出し**（どれも「読めるデータの上から始まり、その終わりを越える」形。`risky_cross` で扱う）：
- `XEND`：トラック 1 のデータの終わり → 隙間。記録 23 で止まった。
- `XLD`：GD-ROM の LD リードアウト − 8 から（データ → 領域の間の隙間）。記録 23 では未到達だが XEND と同じ形。
- `XLO`：リードアウト − 8 から（データ → リードアウト）。同上。
- `XGAP<t>`：開始 − 158 の最初のセクタが読めるとき（GDI のトラック 2：トラック 1 のデータ 748〜 → トラック 2 のプリギャップ）。
  `inline` 以外では 1 セクタ（`GP_XGAP<t>_CHK`）で確かめ、読めたら `GP_XGAP<t>_SKIP ... starts_on_data_...`、読めなければ（CDI の 11694 のように隙間から始まる、記録 23 で 5/21/00 ですぐ返った）そのまま送る。

安全なまま残すもの（記録 23 で問題なし）：`PGL_*`・`PG<t>_*`（1 セクタ）、`XPRE<t>`（プリギャップ → 本体）、隙間から始まる `XGAP<t>`、seq 22 の 1 セクタの各ケース。
`risky_cross=end` なら、すべてのイメージのあとにイメージを順にマウントし直し（情報コマンドだけの試験一式で TOC を読み）、`GP_RISKY_START`、`XGAP<t>`（読めるときだけ）・`XLD`・`XLO`・`XEND` の順。
各読み出しの直前に `GP_RISKY case=.. fad=.. count=.. edge=.. crosses=.. note=may_hang` を書いて記録を送り出すので、止まってもどこで止まったか残る。

**CTRL の試験（`ctl_probe`）**：記録 23 で「トラックの外の CTRL は 0x41」「トラック番号は前の問い合わせのまま」と分かった（150 の音声トラック 1 を読んだあとも 0x41 なので、読んだトラックの CTRL ではない）。
残りは「定数 0x41」か「次のトラック（リードアウトならリードアウト項目）の CTRL」か。TOC から領域ごとに：

| 領域 | 前のトラック / 外の FAD / 次 |
|---|---|
| `GAP` | トラック 1（+16 を読む）/ トラック 2 の開始 − 151 / トラック 2。CD 向け（GDI ではそこもトラック 1 のデータなので `off_track=0`） |
| `HDG` | GD-ROM の LD の最後のトラック（GDI 03 ならトラック 2 = 音声 906、922 を読む）/ LD リードアウト・中央・HD の開始 − 151（1506・23252・44999）/ トラック 3 |
| `LO` | 最後のトラック / リードアウト + 1 / 無し（リードアウト項目の CTRL） |

順方向 `CTLF_<領域>_IN` → `CTLF_<領域>_OFF<k>`、逆方向 `CTLR_<領域>_PRE`（次のトラック、`LO` は最初のトラック）→ `CTLR_<領域>_OFF`（中央）→ `CTLR_<領域>_IN`。読み出しはすべて 1 セクタ・`gap_flags`（0x20、音声は 2,352 B）。
外を読むたびに `GP_CTL`：`ctl`・`scd_ctl`・`scd_rel`、`last_read`（最後に読んだトラックと TOC の CTRL）、`prev_query`（前の REQ_STAT のトラックと CTRL）、`next`、`match=`（`const41`・`last_read`・`prev_query`・`next` / `leadout_entry` のうち合うもの）。
**注意**：GDI 03 の `HDG` は次のトラック 3 がデータ（0x41）なので、「定数」と「次のトラック」は区別できない（`last_read`＝音声 0x01 は区別できる）。
区別できるのは次（またはリードアウト項目）が音声の場所だけで、このカードでは**音声だけの CDI 11 のリードアウト**（最後のトラックが音声）。cfg の `#images=3,10,11,4` を使う。

所要は約 2 分（3 イメージ × seq 22 約 20 + seq 23 約 25 + CTL 約 14 ケース × 約 0.35 秒）。新しく送るコマンドは無い。記録のヘッダに `# config risky_cross=.. ctl_probe=..` の行が増えた。
