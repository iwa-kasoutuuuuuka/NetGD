# netgd-send（G2 の PC 側ツール、T1 のみ）

BBA（ブロードバンドアダプタ）経由で、ゲームイメージ（`.gdi` / `.cdi`）を NetGD の SD カードに書き込むための **PC 側の半分**。設計は `docs/design/bba-sd-write.md`（5 章、7 章、8 章）。

> **重要：Dreamcast 側のアプリ `netgd-recv`（G2-3）はまだ存在しない。**
> このツールが作るのは「送る前の準備（`prepare`）」と「結果の読み取り（`collect` / `status`）」だけで、実機では何も書き込まれない。
> 通し動作は G2-3 が出来てから。ここでの試験はすべてハードウェア無しの単体テスト。

## 何をするか

| サブコマンド | 内容 |
|--------------|------|
| `prepare <image> --job JOBDIR [--slot N \| --name NAME] [--chunk-mib 8] [--mode new\|resume\|overwrite] [--mount] [--placement auto\|link\|copy\|direct]` | イメージを検査し、ジョブフォルダ（マニフェスト、`src.txt`、`/pc/` から読める配置）を作る |
| `collect --job JOBDIR` | Dreamcast アプリが書いた `result.ngr` を読み、マニフェストと照合して結果を表示。**成功のときだけ終了コード 0** |
| `status --job JOBDIR` | 何が未実行 / 完了 / 途中か、どこから再開できるかを表示 |

標準ライブラリだけの Python 3（3.10 で確認）。

```
python netgd-send.py prepare E:\games\crazytaxi\disc.gdi --job E:\netgd\dc1 --slot 2
python netgd-send.py prepare E:\games\Skies.cdi --job E:\netgd\dc1 --name SKIES --mount
python netgd-send.py status  --job E:\netgd\dc1
python netgd-send.py collect --job E:\netgd\dc1
python -m netgd_send ...         # tools\netgd-send で実行すれば同じ
```

### `prepare` がすること

- **GDI**：`.gdi` を読み、全トラックファイルの存在、サイズがセクタ長（2048 / 2336 / 2352）の倍数であること、トラック番号が 1..N、LBA が増えること、オフセット列が 0 であること（ファームはこの列を無視する）を検査する。トラックファイル名は **8.3 形式**に付け替え（`Game (Track 1).bin` → `track01.bin`。既に 8.3 のものはそのまま）、`.gdi` の中身も書き換えて `disc.gdi` にする。
- **CDI**：末尾の 8 バイト（DiscJuggler のバージョンとヘッダ位置）を確認する程度の検査。受け付けるのは v3（0x80000005）と v3.5（0x80000006）。v2（0x80000004）はファームが読めないので拒否する。名前は `disc.cdi`。
- **フォルダ名**：`--slot N` は本家 GDEMU / GDmenu / openMenu 互換の番号フォルダ（`01`〜`99`、100 以上は 3〜4 桁）。`--name` は ASCII だけ（日本語・全角は拒否。ファームが 1 バイト 1 文字で扱うため）。どちらも無ければ、イメージのファイル名（ASCII のとき）から作る。
- **ハッシュ**：ファイルごとの CRC32（zlib と同じ）をマニフェストに書く。あわせて `--chunk-mib`（既定 8 MiB = `NETGD.CKP` の間隔）ごとの「その位置までの CRC32」を `manifest.ngc` に書く。`collect` が再開位置の照合に使う。
- **配置**：元のイメージはコピーしない。ジョブフォルダにハードリンク（同じドライブのとき）を作り、`src.txt` に `/pc/...` のパスを 1 行ずつ書く。リンクできず、元のパスも空白や日本語を含むときはエラーにする（`--placement copy` で回避できる）。`/pc/` のパスにはドライブ名が無いので、`auto`・`direct` でイメージがジョブフォルダと別のドライブにあるときもエラーにする（`--placement copy` を使う）。

ジョブフォルダの中身：

```
JOBDIR\job.ngj             操作の並び（statfs / put NNNN モード / finish ...）
JOBDIR\0001\manifest.ngm   マニフェスト（設計 5.1）
JOBDIR\0001\manifest.ngc   チャンクごとの累積 CRC32（PC 側の照合用）
JOBDIR\0001\src.txt        manifest の file 行と同じ順の /pc/ パス
JOBDIR\0001\disc.gdi ...   ハードリンクと書き換え済みの disc.gdi
JOBDIR\result.ngr          Dreamcast アプリが書く結果
```

同じ `JOBDIR` に別のイメージで `prepare` すると 0002、0003 ... と追加される。同じ送信先フォルダで `prepare` し直すと、その番号が更新される（モードを `resume` に変えるときなど）。前回の `result.ngr` は `result.prev.ngr` に退避する。

### `collect` の終了コード

| コード | 意味 |
|--------|------|
| 0 | 全 put が `ok`（`FILE` 行があれば CRC32 とサイズがマニフェストと一致）で、`finish` も成功 |
| 1 | 失敗（`NGD:ERROR`、CRC / サイズの不一致、再開位置の CRC 不一致、壊れた結果行、`finish` の失敗） |
| 2 | 未完了（結果が無い、途中で止まった、`finish` に届いていない）。**何も失われていない。`--mode resume` で続きから** |

失敗や途中のときは、止まったファイルと「次に再開できる位置（チェックポイント、チャンクの境界）」を表示する。

## T1 の流れ（ユーザーの Dreamcast-Dev-Tools-for-BBA と）

1. `dc-tool-ip` を起動するドライブと、イメージ・ジョブフォルダを**同じドライブ**にする（`/pc/` の規則。ハードリンクも同一ボリュームが必要）。パスに空白と日本語を入れない。
2. `integrations\dcdevtools\netgd.ps1` を BBA ツールの `profiles\` にコピーする（またはフルパスで指定）。
3. 環境変数を設定して `dc_auto` を実行する。

   ```
   $env:NETGD_IMAGE = 'E:\games\crazytaxi\disc.gdi'
   $env:NETGD_SLOT  = '2'
   .\scripts\dc_auto.ps1 -Preset netgd
   ```

   - `Prepare`：`netgd-send.py prepare ...`（ジョブフォルダを作る）
   - 実行：`netgd-recv.elf` が dcload-ip 経由で起動し、`/pc/` から `src.txt` のファイルを 64 KiB ずつ読んで SD に書く。進捗は `NGD:` 行として `dc_auto` のログと `dc_gui` に出る
   - `PostRun`：`netgd-send.py collect --job ...`（成否を表示）
   - `ReloadReq`：`JOBDIR\reload.req`。アプリは 1 秒に 1 回これと `cancel.req` を見て、チェックポイントを残して `exit(0)` する
4. 途中で止まったら、そのままもう一度実行する（既定の `NETGD_MODE` は `resume`）。アプリが `NETGD.CKP` から続きを書く。
5. 完了後、SD の `JOBDIR` で指定した番号フォルダが再マウントされる（`--mount` / `NETGD_MOUNT=1` のとき）。

送信・実行・ログ・noexec の検出は **すべてユーザーの既存ツール（dc_auto / dc_gui / dc-tool-ip）の仕事**で、`netgd-send` は dc-tool-ip を起動しない。**dc-tool-ip を強制終了しない**（noexec になり、電源の入れ直しが必要になる）。中断したいときは `cancel.req` を作る。

## 結果ファイル `result.ngr`（`collect` が読む形式）

設計 5.3 の `NGD:` 行と同じ書式。1 行 1 項目、LF 区切り。最後の行が改行で終わっていなければ（4 KiB 書き込みの途中で止まった）無視する。

```
NGD:HELLO app=1.0.0 fw=OGDM/3 caps=7 free=... total=...
NGD:RESUME 0001 file=1 offset=8388608 crc=0123ABCD     再開したとき（どこから、その位置までの CRC32）
NGD:PUT 0001 file=3/4 bytes=104857600/1185760800 rate=912000
NGD:FILE 0001 file=1 size=1234 crc=89ABCDEF            ファイルごとの検証結果（任意）
NGD:DONE 0001 ok
NGD:ERROR 0002 code=ENOSPC need=734003200 free=10485760   （チャンクの位置が分かれば file=N offset=N を付ける）
NGD:FINISH ok mounted="02"
NGD:END ok=1
```

`RESUME` / `FILE` / `END` は設計の追加提案（G2-3 の実装時に合わせる。設計のギャップとしてコードに `DESIGN-GAP` で記してある）。

## 安全の決まり（設計 5.5 より。アプリが守る。PC 側は前提として検査する）

- 書くのはカードの**第 1 パーティションの FAT32 だけ**。FAT32 でなければ何もせずエラー。フォーマット機能は無い。
- `GDEMU.CFG` が指す既定のフォルダ、アプリ自身の起動元フォルダ、`01`（GDmenu の慣習）は、明示の強制指定が無い限り上書き・削除しない（`EPROTECTED`）。
- `delete` は `NETGD.MF` か `.gdi` / `.cdi` / `.NGP` があるフォルダだけ。ルートは消せない。
- 空きが「必要量 + 16 MiB」に満たなければ `BEGIN` の時点で `ENOSPC`。確保が先に終わるので、転送の途中で満杯にならない。
- 連続領域が取れなければ既定で `EFRAG`。断片化を許すのは明示の指定のときだけ。
- 記述子は `.NGP` の拡張子で書き、最後の 3 バイトの書き換え 1 回で `.gdi` / `.cdi` に確定する。転送の途中でどこで電源が落ちても、未完成のイメージはメニューに出ない。
- PC 側：フォルダ名・ファイル名は ASCII のみ、127 バイト以下、`\ / : * ? " < > |` なし。1 ファイル 4 GiB 未満。ファイルは 128 個まで。
- dcload-ip は認証も暗号化も無い。信頼できる LAN だけで使う。`/pc/` への書き込みは結果ファイルだけ（4 KiB ずつ + `fflush`）。

## `push` と `remote`（BBA 経由、NetGD メニューへ。T2 の PC 側）

`netgd-menu`（`tools/netgd-menu`、設計 `docs/design/menu-app.md` 8 章）の取り込み画面に、ジョブフォルダ無しで直接送る。**実機では一度も動かしていない**（ホスト試験は、Python の模型とメニュー中核の `menu_host serve` の両方に対して通る）。

1. Dreamcast：メニューの一覧で **Y**。画面に `IP 192.168.0.20  port 20039  code 4821` が出る。
2. PC：

```
python netgd-send.py push game.gdi --host 192.168.0.20 --code 4821 --slot auto
python netgd-send.py push game.cdi --host 192.168.0.20 --code 4821 --name MYGAME --title "My Game" --mount
python netgd-send.py remote list --host 192.168.0.20 --code 4821
python netgd-send.py remote delete 05 --host 192.168.0.20 --code 4821
```

- `push IMAGE`：`prepare` と同じ検査（`image.plan_image`）をして、元のファイルから直接読んで送る（コピーもリンクも作らない。書き換えた `.gdi` はメモリ上）。マニフェストに **`name.txt`**（メニューに出るタイトル）を 1 ファイル足す。中身は `--title`、無ければイメージの隣の `name.txt`、それも無ければ元のファイル名の拡張子を除いたもの（ASCII 以外は `?`）。
- `--slot N|auto`：`auto` は先に `LIST /` を送り、使われている番号フォルダを避けて 2 以上で最小の空き番号にする（`01` はメニュー自身）。`--name` と `--slot` のどちらも無ければ `auto`。
- `--mode new|resume|overwrite`：既定は `resume`（中断したら同じコマンドをもう一度で続きから）。`--mount`：終わったらそのフォルダをマウント。`--force`：保護フォルダ（`GDEMU.CFG` の既定・`01`）への上書き。**本体の画面でも A を 1 秒押し続けて許可する必要がある**。`--allow-fragmented`、`--no-finish`（セッションを閉じない）。
- 終了コードは `collect` と同じ：0 成功、2 途中（本体で B を押した、LAN が切れた：同じコマンドで再開できる）、1 失敗。
- `remote statfs|list [フォルダ]|delete フォルダ [--force]|verify フォルダ|finish [--mount フォルダ|--none]`：単発の操作。`NGD:` の行がそのまま出る。
- 認証は 4 桁の合言葉だけ（誤操作よけ。暗号化は無い。LAN 用）。3 回間違えると本体が合言葉を作り直す。
- プロトコルの定義は `netgd_send/ngtp.py`（標準ライブラリだけ）。
- **`--host` を省略すると探索する**（`push`・`remote` とも）：UDP 20039 に `NGTP-DISCOVER 1` を送り（既定の宛先は `255.255.255.255`）、メニューの `NGTP-HERE 1 name=NetGD app=... state=idle|busy` を 2 秒待つ。答えが 1 台ならそれを使い、0 台・2 台以上なら `--host` を求めて終了コード 1。合言葉（`--code`）は探索では分からないので、いつでも要る。見つからないとき（ルータの設定でブロードキャストが届かない、PC に複数の NIC がある）は `--broadcast 192.168.0.255`（そのネットワークのブロードキャストアドレス）か本体の IP を指定する。`--discover-timeout` で待ち時間を変える。
- `discover`：探索だけして一覧を出す（`python netgd-send.py discover [--broadcast ADDR] [--port P]`）。見つかれば 0、無ければ 1。本体の取り込み画面（Y）が開いている間だけ答える。
- **箱絵**：`push --art 絵.png` で、ゲームのフォルダに `NETGD.ART`（128×128、RGB565、32,784 バイト。形式は `tools/netgd-menu/README_ja.md` の実装メモ）を一緒に書く。縦横比を保って 128×128 に収め（余白は黒）、メニューが一覧の右に出す。`絵.png` の代わりに作り済みの `NETGD.ART` も渡せる。`python netgd-send.py art 絵.png -o NETGD.ART` はファイルを作るだけ（SD カードのフォルダへ自分で置く用）。**Pillow は任意**：入っていれば JPEG・BMP など何でも読む（LANCZOS で縮小）。無ければ内蔵の PNG 読み取り（8／16 ビットの灰色・RGB・RGBA・パレット。インターレース無し）と箱形平均の縮小を使う（`--no-pillow` で強制）。試験は Pillow 無しでも通る。

## テスト

```
cd tools\netgd-send
python -m unittest discover -s tests -v
```

合成した小さな GDI（3 トラック、2352 / 2048 バイトセクタ）と CDI で、マニフェストの決定性、CRC32 のテストベクタ、8.3 名（衝突・非 ASCII の拒否）、チャンク境界、`collect` の成功 / 途中 / 破損 / 再開 / 失敗を確かめる。テストの一時フォルダは `tests\_work_*`（終了時に消える）。ファイル操作が遅い環境（ウイルス対策ソフトの走査）では 30 秒ほどかかる。

## 未実装・今後

- T2 の探索は実装済み・実機未検証（`discover`、`--host` の省略）。複数の NIC の PC では各 NIC のブロードキャストへは送らないので、`--broadcast` が要ることがある。
- `prepare` が作る `job.ngj` は `statfs` と `put` と `finish` だけ（`list` / `delete` / `verify` の単発は T2 の `remote` で行う）。

`prepare` は前回の `result.ngr` で `DONE ok` だった項目を `job.ngj` から外す（再送を防ぐ）。失敗・未完の項目は残る。.gdi のトラック名は UTF-8、ANSI コードページ、latin-1 の順に読み、ディスク上にあるファイルに当たった読み方を採る。
