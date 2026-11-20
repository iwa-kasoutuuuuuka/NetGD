# mksdcard：純正 GDEMU 調査用の SD カードの中身を作る

`tools/gdprobe` で純正 GDEMU 6.0（のちに NetGD）を調べるための SD カードの中身を、ルート直下の番号フォルダごと作る。
Python 3 の標準ライブラリだけで動く。**同じ引数なら毎回まったく同じバイト列**ができる（再現できる）。

作られるイメージはすべて**このツールが生成した合成データ**で、市販ソフトの内容も著作物も含まない（法的に問題のあるデータは入らない）。
**起動はしない**（IP.BIN のヘッダは付けてあるので、メニューやカード管理ツールの一覧には出るが、実行しても何も始まらない）。

## 作るもの

| フォルダ | 中身 | 備考 |
|----------|------|------|
| `01` | `--menu` で渡した既存の `01` フォルダ（GDmenu / openMenu）のコピー。無ければ `README.txt` だけ | 普段のカードの `01` を丸ごとコピーする |
| `02` | `--dcload` で渡した dcload-ip の CDI を `disc.cdi` として。無ければ `README.txt` だけ | ユーザーの Dreamcast-Dev-Tools-for-BBA の `scripts\make_dcload_disc.ps1`（DreamSDK の PC）で作る |
| `03` | GD-ROM の GDI（`disc.gdi` + `track01.bin` `track02.raw` `track03.bin`） | track1 データ LBA 0、track2 音声 LBA 756、track3 データ LBA 45000（先頭セクタに IP.BIN ヘッダ） |
| `04` | **2 トラックの CDI**（`disc.cdi`） | セッション1 = 音声1本、セッション2 = データ（LBA 11702、MIL-CD の「audio/data」配置） |
| `05` | **3 トラックの CDI**（`disc.cdi`） | セッション1 = 音声2本、セッション2 = データ（LBA 12254） |
| `06` | 音声トラックの多い GDI | track1 データ、track2〜4 音声（低密度領域）、track5 データ LBA 45000 |
| `07` | **空のフォルダ** | 中身なし（下の注意） |
| `NETGD-PROBE-MANIFEST.json` | 生成したトラックの一覧（種類、LBA の範囲、セクタ長、ファイル、生成パラメータ、トラックごとの CRC-32） | **PC に残す**。SD に入れてもよいが、記録の照合（`compare.py --manifest`）は PC 側でやる |

データセクタの中身：各 2048 バイトの先頭に `NETGD-PROBE img=03 trk=3 lba=00045000 ` という文字列、そのあとは (img, trk, lba) から決まる擬似乱数
（SHAKE-128）。各トラックの先頭セクタが IP.BIN の場合（03・06 のデータトラック、04・05 のセッション2）は、256 バイトの IP.BIN ヘッダ
（`SEGA SEGAKATANA `、`SEGA ENTERPRISES`、製品名 `NETGD PROBE 03` など）が先頭に入り、文字列は 0x100 に移る。
GDI のデータトラックは 2352 バイトの生 Mode 1（同期・ヘッダ・EDC・P/Q の ECC つき、ECMA-130）、
CDI のデータトラックは mkdcdisc と同じ 2336 バイトの Mode 2 Form 1。音声は 2352 バイトの 16 ビットステレオ PCM（小さな鋸波。各セクタの
最初の 1 サンプルが `(左 = lba の下位16ビット, 右 = 上位16ビット)`）。

CDI は KallistiOS 系の **mkdcdisc**（https://gitlab.com/simulant/mkdcdisc 、MIT。`src/disc_image/formats/cdi.c`、`disc_image.c`、`disc_layout.cpp`、
`raw_sector.c`）の書き方に合わせてある：トラックデータを前から並べて pregap 150 も実際に入れ、最後に記述部と 4 バイトの長さ。
データトラックの後ろに 2 セクタの postgap、音声は 302 セクタ以上、セッション長 = リードイン 4500 + リードアウト（6750 / 2250）+ トラック。

サイズ（既定）：03 約 50 MB、04 約 11 MB、05 約 12 MB、06 約 53 MB、合計約 125 MB。`--small`（テスト用）は合計約 17 MB。

## 使い方

```sh
cd tools/mksdcard
python mksdcard.py E:\sdcard_out --menu D:\sd_normal\01 --dcload E:\dcload\disc.cdi
python mksdcard.py E:\sdcard_out --small              # 小さい版（テスト用）
```

- `OUTDIR` が空でない場合は何もせず終了する。上書きしたいときだけ `--force`。
- `--menu` / `--dcload` を省くと、`01` と `02` には「ここに入れる」という `README.txt` だけが入る。**そのままでは起動できない**ので、あとで自分でコピーする。
- 作った直後の確認（NetGD の `discimg` のパーサで読んでマニフェストと突き合わせる。Rust が要る）：

  ```sh
  python verify.py E:\sdcard_out            # common/discimg の examples/inspect を使う
  python tests/test_mksdcard.py             # 単体テスト
  ```

## SD カードに入れる

**注意：diskpart は選んだディスクを丸ごと消す。`list disk` で容量を見て、SD カードの番号を必ず確認する。システムのディスクを選ぶと Windows が消える。**
SD カードは調査用の別の1枚を使うこと（普段使いのカードは消さない）。

管理者のコマンドプロンプトで：

```
diskpart
list disk
select disk N        <- N は SD カード。必ず容量で確認
clean
convert mbr
create partition primary
format fs=fat32 quick
assign
exit
```

（GDEMU は **MBR パーティションありの FAT32** でないと読めない。32 GB を超えるカードは Windows 標準では FAT32 にできないので、小さめのカードを使う。）

そのあと `OUTDIR` の**中身**（`01`〜`07` のフォルダ。`NETGD-PROBE-MANIFEST.json` は入れても入れなくてもよい）をカードのルートにコピーする。

- **`07`（空のフォルダ）はコピーで落ちることがある**（空のフォルダを無視するコピーツールがある）。カードを見て `07` が無ければ、**手で空のフォルダ `07` を作る**。
  中に何も入れない。
- コピー後、カードの `03`〜`06` のファイルサイズが元と同じかを確認する（途中で切れた画像は結果を壊す）。

## 限界・分かっていないこと

- **純正が生成した CDI を受け付けるかは分からない**。mkdcdisc の書き方に合わせたので受け付ける見込みは高いが、受け付けないなら、それ自体が調査結果になる
  （`04` / `05` への切替が失敗する）。比較用に、**動作が確認されている自作ソフトの CDI があれば `08` などに1本入れておく**と切り分けられる（任意。ユーザーが持っている場合）。
- 合成データなので起動はしない。GD-ROM 本来のディスク構造（ISO9660 のファイルシステム、IP.BIN 全体）は作っていない。
- CDI のデータトラックは、セクタ長が 2336（ヘッダなし）。純正がそれを 2048 / 2352 としてどう返すかは、記録（`HASH` 行の `ssz=`）で分かる。
  `compare.py --manifest` は `ssz=2048`（ユーザーデータ）と `ssz=2352`（生セクタ）のどちらも再計算できる。
- mkdcdisc の記述部では、音声が2本以上のとき 2 本目以降の `start_lba` が「pregap の頭」になる（`05` のトラック2は LBA 452）。NetGD のパーサはそれをそのまま
  データの先頭とみなす。純正がどう扱うかも調査の対象。
- CDI の最後には postgap の 2 セクタ（データではない）がある。NetGD の TOC のリードアウトはそれを含むので、gdprobe のシーク試験が最後のセクタを読むと
  postgap に当たる（`compare.py --manifest` では「照合できず」になる）。

## 記録の照合

gdprobe の記録（`HASH` 行は `img=<SD のフォルダ番号>`）を、生成したイメージの期待値と照合する：

```sh
python tools/gdprobe/compare.py --manifest E:\sdcard_out\NETGD-PROBE-MANIFEST.json tools/gdprobe/host/gdprobe_stock_....txt
```

期待値はマニフェストと生成コードから再計算する（イメージのファイルは要らない）。CRC は gdprobe の `hash.c` と同じ CRC-32（`zlib.crc32`）で、
記録の `ssz=2048` はユーザーデータ 2048 バイト、`ssz=2352` は生セクタ全体。不一致は「イメージ・LBA」単位で表示する。

## gdprobe の設定のヒント

- 03 / 06 は IP.BIN が FAD 45150 にあるので、gdprobe の既定のままでよい。
- 04 / 05 のデータトラックは短い（FAD 11852 / 12404 から）。`REQ_SES(2)` が取れない場合は `gdprobe.cfg` に `ip_fad=11852`（04）/ `ip_fad=12404`（05）を書く。
- データトラックは各イメージの**最後**にあるので、gdprobe のシーク試験（IP.BIN の位置からリードアウトまで）の読み出し先はデータトラックの中に収まる。
