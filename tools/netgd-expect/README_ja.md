# netgd-expect：NetGD が gdprobe に何と答えるかを、実機なしで予測する

`tools/gdprobe` の記録（純正 GDEMU 6.0 で取ったもの）と、**NetGD のファームが同じ試験に返すはずの記録**を同じ形式で出して、
`compare.py` で突き合わせる。NetGD を書き込む前に、純正の記録が届いた時点で「NetGD が純正と違うところ」の一覧が出る。

予測の作り方：`common/discimg` の公開 API だけを使う `examples/expect.rs` が、ファーム（`mcu/netgd/src/ide.rs`）と同じ手順で
イメージを読み込み（GDI は各トラックファイルの大きさからセクタ数を出す、境界は「3 番以降の最初のトラック、無ければ 45000」、
セッション数は GDI 1・CDI は記述部の値）、同じビルダ（`atapi::build_toc` / `req_ses_reply` / `req_stat_reply` / `get_scd_reply`）を
`gdprobe/probes.c` と同じ CDB・確保長で呼ぶ。ファームが応答を包む部分は `ide.rs`（HEAD）に合わせてある：

| 項目 | 予測の内容 |
|---|---|
| レジスタ | 正常完了 = `status=0x50 err=0x00 irr=0x03`。CHECK は純正どおり `status=0x11`、`err` = センスキー << 4（0x20 NOT READY、0x50 ILLEGAL REQUEST、0x60 UNIT ATTENTION。D5） |
| 切替直後 | 切替の窓（D13）：TUR は NOT READY `2/3a/00` → `2/04/01` → UNIT ATTENTION `6/28/00` を 1 回 → READY。`SW_POLL` はこの 4 行（時刻は `-`） |
| REQ_MODE | 純正の 32 バイト（`00 00 00 00 00 b4 19 00 00 08` + `SE      ` + `Rev 6.42` + `990316`、R117）の窓。32 バイトを越える窓と長さ 0 は ILLEGAL REQUEST（`err=0x50`） |
| REQ_ERROR | `F0`、バイト 2 = 保留中のセンスキー、バイト 8・9 = ASC/ASCQ（D7） |
| REQ_STAT / GET_SCD | 待機状態（FAD 150、トラック 1）。REQ_STAT バイト 1 の上位 = ディスク形式（CDI 0x2、GDI 0x8。SECNR と同じ）、GET_SCD 形式 1 の位置はバイナリ FAD、形式 0 は Q = 0x40・ほか 0 |
| REQ_SES | `req_ses_reply` どおり（バイト 0 = ドライブ状態、待機中は 0x02）。範囲外のセッションは ILLEGAL REQUEST（`status=0x11 err=0x50`、`bytes=0`） |
| `52 00` | `00 00 00 00 00 00 20 05`（バージョン 5.20.0） |
| CD_READ | CDB のバイト 1 は見ない。常に 1 セクタ 2048 バイト（`ssz=2048`、`0x10` でも 2352 にならない）。データは `file_offset + (LBA − 開始 LBA) × セクタ長 + 16`（2352）/ `+ 8`（2336）の 2048 バイト。要求がトラックの残りを越えると ILLEGAL REQUEST（`err=0x50`、`sense=5/00/00`） |
| `52 82 n`（n が空フォルダ） | 純正どおり受け付ける（`status=0x50`、D12）。同じ窓のあと `2/3a/00` のまま（`SW_RESULT rc=timeout`）。そのあとの `img=7` は「ディスクなし」の試験：TUR・REQ_STAT・GET_TOC・REQ_SES_0・GET_SCD は `0x11/0x20`、REQ_SES_1〜3 は `0x11/0x50`、REQ_MODE・REQ_ERROR・`52 00` は通常どおり、読み出しなし |
| `52 82 n`（n のフォルダが無い） | ILLEGAL REQUEST（`status=0x11 err=0x50`）。メディアは変わらない（純正は未調査。NetGD の方針） |

予測できない項目は `-` で書く：`us=`、`t_ms=`、`poll=`、`polls=`、`changes=`、`min_us=`、`max_us=`、`kib_s=`。`compare.py` は `-` を数値として読めないので、
時間の比較をしない（差分にも注記にもならない）。

予測しないもの：`img=0`（電源投入時にマウントされている dcload-ip のディスク）、`IDENTIFY_PACKET`、`SW_POLL` の回数・時刻。
時間は模擬しない：窓の各段階（150 ms + 150 ms、`discimg::atapi::swap_phase`）を「20 ms ごとに TUR を送る gdprobe ならすべての段階を見る」として、
状態が変わる行の並びだけを書く。段階より粗い間隔のポーリングや、窓の中で TUR 以外を送る場合は表せない。

## コマンド

```sh
# 1 つのイメージ（cargo が要る。初回はビルド）
cd common/discimg
cargo run -q --example expect -- --image E:\sd\03\disc.gdi --img 3 --switch --header --end > exp03.txt
sh tools/netgd-expect/expect.sh --image disc.cdi --img 4           # 同じ。どこからでも呼べる薄い wrapper

# mksdcard のカード全体（03〜07 と、最後の return_to）を 1 つの記録に
python tools/netgd-expect/expect_sd.py E:\sdcard_out --out netgd_expected.txt
python tools/netgd-expect/expect_sd.py E:\sdcard_out --images 3,4,5,6,7 --return-to 2 --no-seek --tp-batches 8
```

`example expect` の主な引数：

| 引数 | 意味 |
|---|---|
| `--image FILE` | `.gdi` または `.cdi`（必須。`--empty-folder`・`--missing-folder` のときは省略可） |
| `--img N` | 記録の `img=` に書く番号（SD のフォルダ番号。既定 0） |
| `--label L` / `--header` / `--end` | ヘッダ行（`# gdprobe ... label=L`）、末尾の `END` 行を付ける |
| `--switch` | `52 82 N` の `SW_CMD` / `SW_POLL` / `SW_RESULT` を先頭に付け、UNIT ATTENTION は消費済みとして試験を書く。無しなら「起動直後の最初の試験」（`TUR_W0` が UNIT ATTENTION、`REQ_ERROR_W0`、`TUR_W1`）。`--ua` / `--no-ua` で明示 |
| `--empty-folder` | そのフォルダはあるが画像が無い（受け付けて「ディスクなし」。`--image` は無視） |
| `--missing-folder` | その番号のフォルダが無い（`52 82 N` を拒否）。`--image` を一緒に渡すと、直前にマウントされていたイメージ（無ければ「ディスクなし」） |
| `--sw-only` | `SW_*` だけ（`return_to` 用） |
| `--ip-fad N`、`--cdread-flags 0x24` | gdprobe の同名の設定（既定 自動判定 / `0x24`） |
| `--no-flags-test`、`--no-throughput`、`--tp-batches N`、`--tp-sectors N`、`--no-seek`、`--seek-n N`、`--seek-span N`、`--no-reads` | 設定の `cdread_flags_test` / `throughput` / `tp_batches` / `tp_sectors` / `seek` / `seek_n` / `seek_span` に対応（既定は gdprobe と同じ） |

`expect_sd.py OUTDIR`：`OUTDIR` の数字のフォルダを `03` 以降を昇順に（`--images` で変更）処理する。`disc.gdi` / `*.gdi` / `*.cdi` があればマウント、無ければ
空フォルダ扱い（`07`）、フォルダ自体が無ければ拒否扱い。`--return-to`（既定 `02` があれば 2）で最後の切替の `SW_*` 行を書き、`END` で終わる。`--bin` か環境変数 `NETGD_EXPECT_BIN` に
ビルド済みの実行ファイルを渡すと cargo を使わない。ほかの引数（`--no-seek` など）は `example expect` にそのまま渡す。
別の作業が `cargo` を動かしていてロックで待たされるときは `CARGO_TARGET_DIR` を別の場所にする。

## 純正の記録との比べ方（これが本題）

1. 純正で `label=stock`、`switch=on`、`images=3,4,5,6,7` で実行する（`tools/gdprobe/README_ja.md`）。**同じ設定で NetGD 側の予測を作る**
   （`tp_batches` などを変えたなら `expect_sd.py` にも同じ値を渡す。違っても `--only-common` で共通部分だけ比べられる）。
2. 差分を出す：

   ```sh
   python tools/gdprobe/compare.py --only-common gdprobe_stock_*.txt netgd_expected.txt
   ```

   `--only-common` は、片方にしか無いプローブ（予測に無い `img=0`、設定の違うスループット行）を差分に数えない。
3. **出てきた差分 = NetGD が純正と違う場所のリスト**。`docs/PLAN.md` の D-1〜D-5 に対応づける：

   | 差分の出る行 | 決めること |
   |---|---|
   | `img=4` の `REQ_SES_2`（純正が 6 バイトを返せば差分） | D-1：2 トラック CDI の REQ_SES(2)、`IP_READ` の位置 |
   | `GET_SCD_1`、`GET_SCD_0` の `data=` | D-2（format 1 の中身）、D-5（R〜W ビット） |
   | `REQ_STAT` の `data=`（バイト 1 の Disc Format など） | D-3 |
   | `img=3`・`img=6` の `REQ_SES_0`（セッション数）、`REQ_SES_2`・`_3` | D-4（GDI のセッション数 1 か 2 か） |
   | `GET_TOC_*` の lead-out・CTRL、CDI の lead-out、`REQ_SES_0` の最初のバイト | D-5 |
   | `REQ_MODE_*`、`REQ_ERROR`、`GDEMU_VERSION` | 純正で確認済みの値のはず。差分が出たら純正のバージョン違いを疑う |
   | `CDREAD_FLAGS` の `ssz` / `crc`（`0x10`） | NetGD は 2048 固定。純正が `0x10` で 2352 を返すなら生セクタ読みを足すか決める |
   | `img=7` の `SW_*` と、その後の行 | 空フォルダへの切替（D12。いまは純正に合わせてある） |
   | `SW_POLL` | 切替の窓の段階（D13。いまは純正に合わせてある） |

   差分が無い行は「いまの NetGD のままで純正と同じ」。`TIMING` の注記は出ない（予測側が `-`）ので、速度の比較は純正の記録同士・NetGD 実機の記録同士で行う。
4. NetGD を書き込んだあと、実機の記録（`label=netgd`）と予測を比べると、**ファームが予測どおりに動いたか**の確認にもなる
   （予測と違えば、ファームかこのツールのどちらかの読み違い）。

## 限界

- 予測は **ファームのソース（HEAD の `ide.rs`）の読み解き**であって、実機では確認していない。とくに、切替の窓の段階と UNIT ATTENTION の出方、
  CD_READ が範囲外で返す CHECK（`status=0x11 err=0x50`）、`REQ_MODE_0_32` の `irr`（コード上は直前のコマンドの値のまま = `0x03` と仮定）は読みの結果。
- GDI は `disc.gdi` の隣のトラックファイルの大きさでセクタ数を決める（ファーム同様）。トラックファイルが足りない・短いと、予測は CHECK になる。
- 純正が mksdcard の CDI を受け付けない場合、純正の記録の `img=4/5` は切替失敗の行になり、予測と大きく違う（それ自体が調査結果）。
- READY までの時間と `SW_POLL` の回数は予測しない（`-`。段階ごとに 1 行を置いてあるだけ）。

## テスト

```sh
python tools/netgd-expect/test_expect.py        # --small の mksdcard カードを作り、cargo で予測を作って検証する（数秒）
GDPROBE_SAMPLE=<C ハーネスの記録> python tools/netgd-expect/test_expect.py   # 偽ドライブの記録との比較も確認
```

確認すること：予測どうしの `compare.py` が差分 0、`compare.py --manifest`（mksdcard の生成コードでセクタを再計算）で `HASH` が全部一致（不一致 0）、
`tests/atapi.rs` と同じ REQ_STAT / GET_SCD / TOC / REQ_SES のバイト、空フォルダ 07 の切替（受け付け・窓・ディスクなしの試験）、無いフォルダの拒否（`err=0x50`）、切替の窓の 4 行、`CDREAD_FLAGS` 4 行が同じ内容、
`--only-common` が片側だけの行を無視すること。

## ファイル

| ファイル | 内容 |
|---|---|
| `common/discimg/examples/expect.rs` | 予測本体（1 イメージ）。`discimg` の公開 API だけを使う |
| `tools/netgd-expect/expect_sd.py` | mksdcard の OUTDIR 全体を 1 つの記録にする |
| `tools/netgd-expect/expect.sh` | `cargo run --example expect` の薄い wrapper |
| `tools/netgd-expect/test_expect.py` | テスト |
| `tools/gdprobe/compare.py` | 小さな追加：`--only-common`（片側だけのプローブを無視。既存の動作は変わらない） |
