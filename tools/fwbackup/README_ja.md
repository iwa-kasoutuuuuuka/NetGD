# fwbackup - GDEMU 6.0 純正ファームの読み出し可否の確認とバックアップ

## 目的

GDEMU 6.0 (クローン) の MCU (ATSAM3U2E、フラッシュ 128 KB) の純正ファームを SWD で読めるかを調べ、読めるなら **何かを書き込む前に** バックアップを取る PC 側ツールです。
**読み取り専用**です。書き込み・消去・リセットはできないように作ってあります (後述)。標準ライブラリだけの Python 3.10 で動きます。

## 法的・倫理的な注意

- 吸い出したファームは **元の作者の著作物**です。**個人のバックアップとして非公開で保管**してください。
- **コミット、アップロード、共有をしないでください。** `.gitignore` で出力先 (`tools/fwbackup/out/`、`gdemu60_flash_*`、`tools/fwbackup/**/*.bin`) を除外してあります。
- 法的助言ではありません (**not legal advice**)。扱いは自己責任で、必要なら各自で確認してください。
- ロックを回避しようとしないでください (グリッチ攻撃、電圧・クロック操作、その他の突破の試みは一切しない)。
- dump のファイル名を変えたり、クラウド同期フォルダ (OneDrive、Dropbox 等) に置いたりしないでください (`.gitignore` が除外するのは `gdemu60_flash_*` だけです)。
- NetGD は dump の中身を使いません (流用しません)。目的は「純正に戻せる保険」だけです。
- Claude に送るのは **`check` の出力だけ**です。dump ファイルは送らないでください。

## 事前準備

- Raspberry Pi Pico に debugprobe (**2.2.3 以上**、CMSIS-DAP v2) が入っていること。
- probe-rs は **ちょうど 0.32.0** を入れる (このバージョンだけソースを安全確認済み。他のバージョンは `UNREVIEWED_PROBE_RS` で止まり、上書きオプションはありません)。
- `.probe-rs.toml` / `.json` / `.yaml` / `.yml` が、カレント・ホーム・probe-rs の実行ファイルのあるフォルダに**無いこと**を確認する (probe-rs は `[presets.default]` などで `connect-under-reset` を暗黙に足せるため。見つかるとツールも実行を拒否します)。
- 配線は実機作業の作業 2 のとおり **SWDIO / SWCLK / GND の 3 本だけ**。RESET・ERASE・電源 (3V3) は繋がない。
- **Dreamcast は BIOS メニューで止めた状態で実行**する (ディスクアクセス無し)。後述のとおり、読み出し中は attach のたびにコアが短時間 halt します。
- **GDEMU 基板は Dreamcast から給電**して、Dreamcast の電源を入れた状態で実行します (プローブから給電しない)。
- Windows では debugprobe に WinUSB ドライバが必要な場合があります (`probe-rs list` に出なければ Zadig 等で)。

## 使い方

```bash
cd tools/fwbackup
python fwbackup.py check                       # まずこれだけ。何も書き出さない
python fwbackup.py dump --out out              # check が UNLOCKED_2E のときだけ。約 1〜3 分
python fwbackup.py inspect out/gdemu60_flash_281A0761_<日時>.bin
python fwbackup.py inspect <file> --strings    # 自分の解析用 (共有しない)
```

プローブが複数あるときは `--probe 2e8a:000c[:シリアル]`。別の場所の probe-rs は環境変数 `FWBACKUP_PROBE_RS` にフルパス (実行ファイルのパスだけ。JSON リスト形式はテスト専用)。
`dump` は 4096 バイトずつ全体を **2 回**読み、チャンクごとに比較します。違ったチャンクは最大 5 回読み直して多数決 (3 票以上かつ他が 1 票以下) で決め、決まらなければ**何も書き出さず**に失敗します (終了コード 3)。
成功すると `gdemu60_flash_<CIDR>_<UTC日時>.bin` / `.sha256` / `.json` (CIDR、EXID、範囲、probe-rs のバージョン、時刻、読み回数、チャンクごとの CRC32) ができます。**既存ファイルは決して上書きしません** (終了コード 4)。

## `check` の結果の読み方

| RESULT | 終了コード | 意味と次の手順 |
|---|---|---|
| `UNLOCKED_2E` | 0 | CIDR が `0x281A0761`。読める。`dump` でバックアップを取る。 |
| `UNLOCKED_OTHER` | 10 | 別の SAM3U (例: 4E = `0x28100961`) か別の MCU。`dump` は拒否。出力を Claude に送る。 |
| `LOCKED_OR_NO_ACCESS` | 11 | SWD の DP は見えるが読めない = セキュリティビット (GPNVM0) の可能性大。**ここで止める。** |
| `NO_TARGET` | 12 | 何も応答しない。Dreamcast の電源、GND/SWDIO/SWCLK、JTAGSEL (ローであること)、他ソフトがプローブを掴んでいないか。**配線も電源も確実に正しいのにこの結果なら、セキュリティビットで SWD 自体が無効になっている可能性もある (配線不良と区別できない)。ERASE には触らず、出力を Claude に送る。** |
| `NO_PROBE` / `NO_PROBE_RS` / `MULTIPLE_PROBES` / `BAD_CHIP_NAME` | 2 | プローブ未検出 (すべて `(inaccessible)` の場合を含む。WinUSB ドライバを確認)、probe-rs 無し、プローブ複数 (`--probe` 指定)、チップ名が古い probe-rs に無い。 |
| `OLD_PROBE_RS` | 2 | probe-rs が `unexpected argument` 等でオプションを拒否した。0.32.0 を入れる。 |
| `UNREVIEWED_PROBE_RS` | 2 | 0.32.0 以外。安全確認済みなのは 0.32.0 だけ。ちょうど 0.32.0 を入れる。 |

CIDR が ATSAM3U2E と同系列 (`0x281A0760`〜`0x281A077F`) で `0x281A0761` ちょうどでないものは「ATSAM3U2E のリビジョン違い」と表示しますが、安全側に倒して `UNLOCKED_OTHER` (dump 拒否) として扱います。

### ロックされていた場合

データシート 8.2.3.5 / 20.4.3.6: セキュリティビット (GPNVM bit 0) が立っていると、ICE (JTAG/SWD) からのフラッシュ・SRAM・コアレジスタ・内部周辺へのアクセスは**すべて禁止**です。
解除は ERASE ピンでフラッシュ全体を消すしかなく、**純正ファームは二度と戻りません。** 純正を捨てると決めるまで ERASE には触らないでください。
このツールは ERASE を一切行いません。`check` の出力を Claude に送ってください。

## 読み取り専用であること (安全策)

`fwbackup.py` の probe-rs 呼び出しは、すべて `ProbeRs.run()` の許可リストを通ります。許可は `list`、`info`、`read` (と単独の `--version`) だけです。
次は実行前に拒否されます (ユニットテストあり): `download` `erase` `write` `run` `reset` `attach` `gdb` `serve` など他の全サブコマンド、`--connect-under-reset`、`--allow-erase-all`、`--cycle-power`、`-o/--output`、`--format`、`--chip-description-path`、SWD 以外のプロトコル、フラッシュ範囲と CHIPID レジスタ (`0x400E0740`〜`0x400E0747`) 以外へのアドレスの `read`。
さらに、環境変数 `PROBE_RS_*` (`PROBE_RS_CONNECT_UNDER_RESET` など clap が環境変数から読むもの) は子プロセスに渡しません。
probe-rs は `.probe-rs.{toml,json,yaml,yml}` をカレント・実行ファイルのフォルダ・ホームから読み、`[presets.default]` で黙ってオプションを足せます (環境変数の除去では防げない)。そのため、これらのファイルが 1 つでもあると**実行を拒否**し、さらに probe-rs は毎回新しい空の一時フォルダをカレントにして起動します。
probe-rs の `download` / `erase` / `reset` などを**手で実行してはいけません**。`SAM3U.yaml` には `atsam3u_gpnvm` というフラッシュアルゴリズムまで入っており、GPNVM (セキュリティビット) を書き換えられてしまいます。
`dump` は `<名前>.bin.part` に排他的に作って flush + fsync し、読み戻して SHA-256 を照合してから最終名へリネームします (既存ファイルは上書きされず、不一致なら `.part` を消して終了コード 3)。
`dump` の `--chunk` は 4 の倍数で 256〜65536 (範囲外は終了コード 2)。
**バックアップに含まれないもの**: GPNVM ビット、ユニーク ID、ユーザーシグネチャ (読むには EEFC コマンドが必要で、意図的に行っていません)。フラッシュの内容だけです。

## probe-rs について確認したこと (ソース: GitHub master、v0.32.0)

- `read` の構文は `probe-rs read [--chip C] [--probe VID:PID] [--protocol swd] <b8|b16|b32|b64> <アドレス> <ワード数>`。標準出力は既定で hex-table: `{アドレス:08x}: ` に続けてワードを空白区切り、各ワードは**上位バイトから**表示 (リトルエンディアン値のバイト列を逆順)。b32 は 1 行 32 バイト (8 ワード)。
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs-tools/src/bin/probe-rs/cmd/read.rs
- **`read` は attach のたびにコアを短時間 halt する** (「halt しない」ではない)。`Session::new` → `clear_all_hw_breakpoints` → `halted_access` で 1 回、drop 時にもう 1 回 halt し、再開時にシングルステップし、最後に DHCSR のデバッグ有効ビットを落として DEMCR=0 を書く。リセットも、フラッシュ / EEFC / RAM への書き込みも無い。
  halt は 1 回あたり数 ms ですが、dump 全体では約 260 回 attach するので、**Dreamcast は BIOS メニューで止めて (ディスクアクセス無し) 実行**してください。止まったままになったら Dreamcast の電源を入れ直す。実機では未確認 (probe-rs 0.32.0 のソースによる確認)。
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs/src/session.rs
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs-tools/src/bin/probe-rs/util/common_options.rs (`--connect-under-reset` 等のオプションと `env = PROBE_RS_...`)
- `list`: `The following debug probes were found:` の後に `[N]: <名前> -- VID:PID:シリアル (種別)` (無ければ `No debug probes were found.`)。`info` は `--verbose` で `Debug Port: DPv1, Designer: ...` のツリーを出し (DP が見えている印)、無しでは `Detected chip: <名前>`。失敗は標準エラーと非 0 の終了コード (anyhow の通常の挙動)。
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs-tools/src/bin/probe-rs/cmd/list.rs
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs-tools/src/bin/probe-rs/cmd/info.rs
- チップ名 `ATSAM3U2E` は有効: フラッシュ `0x80000-0xA0000`、RAM `0x20000000-0x20004000`、アルゴリズム `atsam3u_128`。
  - https://github.com/probe-rs/probe-rs/blob/master/probe-rs/targets/SAM3U.yaml
- 古い probe-rs (0.2x) では `info --verbose` や出力形式が異なる可能性があるため、パーサーは「アドレス無しの行」「大文字」「CRLF」「ログ行の混入」にも耐え、最後に総バイト数を検証します。

## 不確かな点

- 実機の GDEMU 6.0 + 実際の probe-rs での動作は未確認 (テストは偽の probe-rs による)。特に、ロック時に `read` が失敗する形 (attach 失敗か読み出しのフォールトか) と `info` の出力は想定。どちらでも `LOCKED_OR_NO_ACCESS` / `NO_TARGET` の判定は `info` の `Debug Port:` の有無で行います。
- 各 `read` は毎回 attach します (1 回あたり約 1 秒、dump 全体で約 260 回)。
