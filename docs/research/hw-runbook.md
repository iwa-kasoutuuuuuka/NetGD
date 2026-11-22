# 実機ランブック（純正 GDEMU 6.0 + Dreamcast + BBA）

作成：2026-11-23（`main` a59cbd00 から切った `docs/hw-runbook`）。**ここに書いた 5 本の実行は、まだ 1 回も実機で行っていない。** 期待する値・画面は、ソースと既存の記録から読んだ「見込み」であって結果ではない。確認できなかったことは **要確認** と付けた。

前提：ユーザーの GDEMU 6.0 の MCU はロックされた扱いなので、NetGD 自体はまだ動かせない。ここの実行はすべて **純正 GDEMU のまま**、Dreamcast + BBA（PC 192.168.10.1、dcload-ip 192.168.10.2）で行う。SWD 端子と ERASE パッドには触らない。

## 0. 先に読む：順番と注意

### 共通の約束

- **一度に 1 回だけ**。`dc_auto` を 2 つ同時に走らせない。前の実行の `dc-tool-ip` が残っている（stale）と、次の実行は待たされる（`dc_auto.ps1` は「Dreamcast がまだ前のプログラムを動かしている → リセットを待つ」と表示して止まる）。残ったら次のとおり。
  1. `Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Stop -Ip 192.168.10.2`（この Dreamcast 向けの `dc_auto` と `dc-tool-ip` を止める。`-Stop` は `dc_auto.ps1` にある）
  2. Dreamcast の電源を切って 5 秒待ち、入れ直し、GDmenu で SD の `02`（dcload-ip）を選ぶ。
- **純正 GDEMU が固まったら、Dreamcast の電源スイッチを OFF にする。リセットボタンでは GDEMU は電源が入れ直されない**（`gdprobe` の README と LOG R183 の「電源の入れ直しが要る」）。OFF → 5 秒 → ON → `02` を選ぶ。
- `dc-tool-ip` はカレントディレクトリを NetGD リポジトリと**同じドライブ（H:）**にして動かす（`/pc/` の規則）。下のコマンドはすべて最初に `Set-Location H:\DC\tools\dc-bba-tools` する。
- プリセットとビルドは **`H:\OpenGDEmu`（main）のもの**を使う。ワークツリー（`H:\OpenGDEmu-seq25` など）のコピーは使わない。ビルドの成果物（`*.elf`、`host\` の記録）は `H:\OpenGDEmu\tools\...` に出る。
- すべて Windows PowerShell 5.1 の 1 行。`dc_auto.ps1` は `H:\DC\tools\dc-bba-tools\scripts\dc_auto.ps1`（実在を確認）。使うパラメータ：`-Preset <プリセットのフルパス>`、`-Elf`、`-NoBuild`、`-Ip`、`-Stop`（いずれも `dc_auto.ps1` の param にある）。
- `dc_auto` のログ：`H:\DC\tools\dc-bba-tools\logs\auto_<日付>_<時刻>.txt`。クラッシュ時は同じ場所に `*.crash.txt`。実行中は `logs\dc_auto_status.txt` に状態が出る。

### 実行順の推奨と、純正が固まる可能性

| 順 | セクション | 純正 GDEMU が固まる可能性 | 固まったら |
|---|---|---|---|
| 1 | §2 g2netbench_tcpfix | 無い（GD-ROM に触れない：`INIT_CDROM` 無し、G1・ATA 呼び出し無し） | — |
| 2 | §3 g2netbench（g2x） | 無い（同上） | — |
| 3 | §4 netgd-t2（`T2_NO_DRIVE=1`） | 無い（ドライブに 1 パケットも送らない） | — |
| 4 | §5 netgd-menu 初回確認（純正） | 低い（ドライブに送るのは `52 00` 1 個、UNIT ATTENTION のときだけもう 1 個。`52 00` は gdprobe が純正に送って記録済みの CDB） | 電源 OFF |
| 5 | §1 gdprobe seq 25c | **ある**（DMA の CD_READ。純正は seq 25b で 2 回目に固まった） | **電源 OFF** → 5 秒 → ON |

- 理由：固まる可能性が高いものを最後にする。逆に 25c を先に回すと、固まった GDEMU が G1 バスの割り込みを出し続けて、ネットの計測に影響するかもしれない（g2bench の README は「固まったあとは電源を入れ直してから測る」と書いている。推測）。
- 25c を先に回したい理由があるなら（記録の価値が最も高い）、その直後に必ず電源 OFF → ON してから他を回す。セクション番号は依頼の順で、実行順ではない。
- 1 回ごとに：結果を回収 → `dc_auto` が終了（`dc-tool-ip` が残っていない）を確認 → 次へ。

### 共通の SD の状態

| フォルダ | 中身 | 使う実行 |
|---|---|---|
| `01` | GDmenu か openMenu（普段どおり） | すべて（ここから `02` を選ぶ） |
| `02` | dcload-ip のディスク（固定 IP 192.168.10.2） | すべて |
| `03` | GDI（`tools/mksdcard` の合成イメージ） | §1 |
| `04` | 2 トラックの CDI（音声 + データ、mksdcard） | §1 |
| `11` | 音声だけの CDI（`mksdcard --set variants`、LOG R148） | §1 |

出典：`docs/research/stock-gdemu-flow.md` ③、`tools/mksdcard/README_ja.md`。§2〜§5 は `01` と `02` だけで足りる（イメージを読まない）。**捨てても困らない SD カード**で行う。

---

## 1. gdprobe seq 25c（G1 DMA の CD_READ、長さちょうど）

### 何を確かめるか

純正の G1 DMA の読み込みの、まだ分かっていない部分。seq 25b（記録 `stock_20261123_26_dma-exact-floor.txt`、LOG R183）で分かったこと：音声 1 セクタを長さちょうど（2,352 B）で DMA すると終わり（`dma_done=1`、`extent=2368` = 要求より 16 B 先まで書く）、32 B に切り下げた（2,336 B）DMA は終わるがドライブが BSY のままになった。25c は `exact` だけで次を測る（`tools/gdprobe/README_ja.md` の seq 25c の節）。

- 音声 1/2/7/16 セクタの `exact`：`extent`、はみ出し `over_n=`、`over_hex=`、次のセクタとの一致 `over_eq_next=`（`DM_<ケース>_OVER` 行）
- データ 1/2/7/16 セクタと、データのスループット（`DM_TP_*`）
- NetGD のホスト側が「要求長を超えて次の 32 B 境界まで書く」前提を作る材料になる

### 準備

SD：§0 の表のとおり（`03`・`04`・`11` が要る）。cfg の置き方（main の `tools\gdprobe` を使う）：

```powershell
Copy-Item H:\OpenGDEmu\tools\gdprobe\host\gdprobe.cfg H:\OpenGDEmu\tools\gdprobe\host\gdprobe.cfg.bak_before25c; Copy-Item H:\OpenGDEmu\tools\gdprobe\gdprobe.cfg.seq25c.example H:\OpenGDEmu\tools\gdprobe\host\gdprobe.cfg -Force
```

- 確認済み：`gdprobe.cfg.seq25c.example` は main の `H:\OpenGDEmu\tools\gdprobe\` にある。`feat/gdprobe-seq25` との `tools/gdprobe` の差は `tests/test_all.c` の 1 行だけ（a59cbd00 と比べた）なので、`H:\OpenGDEmu-seq25` は使わない。
- 確認済み：プリセット `H:\OpenGDEmu\tools\gdprobe\profiles\gdprobe.ps1` は自分の置き場所の 1 つ上（`...\tools\gdprobe`）をフォルダとし、`build.ps1`（`$PSScriptRoot\host` を `/pc/` 側の保存先として ELF に埋め込む）を使う。つまり cfg は `H:\OpenGDEmu\tools\gdprobe\host\gdprobe.cfg` から読まれ、記録も同じ `host\` に出る。`build.ps1` は cfg が**無いときだけ**雛形を作るので、上のコピーを先にやる。
- cfg の中身（確認済み）：`label=stock`、`seq=27`、`images=3,4,11`、`return_to=2`、`dma_modes=exact`、`dma_throughput=on`。seq 21〜24 の試験はすべて off。
- ビルド：プリセットが実行時に `build.ps1 -BbaTools $DevkitRoot` を呼ぶので、別に作らなくてよい（作るなら `Set-Location H:\OpenGDEmu\tools\gdprobe; .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools`）。ビルドで KOS の警告・エラーが出たら全文を送る。

### 安全なところと、固まる危険

- 安全（README より）：読むだけ。DMA 付きで送れるのは CD_READ（0x30）だけ（許可リストと `gd_packet_dma` の二重）、1〜16 セクタ、すべての待ちに上限（`dma_timeout_ms=3000`）、どの出口でも DMA 停止・ASIC bit 消去・キャッシュ破棄。完了しない DMA は、そのケースの種類だけを `DM_STOP` で止める。3 回続けて応答が無ければ `ABORT` を書いて終了する。書き込み・ファーム更新に当たるコマンドは許可リストに無い。
- 25c で `floor` と `pad`（固まった 2 つ）は cfg に書いていないので走らない（`dma_modes=exact` のみ。README：明示した場合だけ、最後に隔離して実行）。
- **それでも固まる危険はある**：`exact` の 2・7・16 セクタは未経験（1 セクタだけが実機で終わった）。ドライブが BSY のまま応答しなくなると、以降の REQ_STAT・TUR も BSY で返り、`ABORT reason=drive-not-answering` で終わる（seq 25b がそうだった）。記録は書き出されるが、`return_to=2`（dcload のディスクへ戻る切替）は効かない。ドライブ側から回復する命令は送らない。
- **固まったら（`[gdprobe] finished (ABORTED: the drive stopped answering)` が出た、または何も進まない）**：Dreamcast の電源スイッチを OFF → 5 秒 → ON。リセットボタンでは足りない。そのあと `dc_auto.ps1 -Stop` で PC 側の残りを止める（§0）。
- 7 セクタ exact は 16,464 B（32 の倍数でなく、1 セクタと同じく 16 B 余る）なので、1 セクタと同じに終わる見込みだが、これは**推測**。2・16 セクタは 32 の倍数（4,704 B・37,632 B）。

### コマンド

```powershell
Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu\tools\gdprobe\profiles\gdprobe.ps1 -Ip 192.168.10.2
```

Dreamcast：電源 ON → GDmenu → `02`（dcload-ip）を選んでから実行する。所要は約 3 分と見込む（seq 25 の見積もり。25c は `floor`/`pad` が無いぶん短いかもしれない。要確認）。

### TV に出るもの

gdprobe は画面に描かない（`main.c` に bfont・video の呼び出しが無いことを確認）。TV は dcload-ip の画面のまま（**要確認**）。進み具合は PC のコンソールの `[gdprobe] ...` と各プローブの行（先頭 150 文字）で見る。終わると `[gdprobe] finished. Returning to dcload.` が出て、dcload-ip の画面に戻る。

### ログ・出力の場所

- 記録：`H:\OpenGDEmu\tools\gdprobe\host\gdprobe_stock_seq00027.txt`（Dreamcast に有効な時計が無いとき `seq<番号>`、`seq=27`。時計があれば日付入りになる）
- `dc_auto` のログ：`H:\DC\tools\dc-bba-tools\logs\auto_<日付>_<時刻>.txt`
- 実行後に `dc_auto` が `[gdprobe] newest record: <パス>` を出す。

### 合否

- 正常終了：記録の最後が `img=2 probe=END abort=0`（return_to=2 の戻り先）、`# done abort=0`。ヘッダに `# config ... dma_modes=exact`。
- 合格（見込み、結果ではない）：
  - 音声 1/2/7/16 の `DM_A<n>_exact` がすべて `dma_done=1`、PIO と一致（`same=1` または `crc` 一致）
  - 1 セクタは seq 25b と同じ `extent=2368`（`over_n=16`）
  - `DM_A<n>_OVER` 行に `over_n=`・`over_hex=`・`over_eq_next=` が出る
  - `DM_TP_SUMMARY` が出る
- 不合格・判断が要るもの：どれかの `dma_done=0`（`DM_STOP slot=audio_exact` が出る）、`ABORT reason=drive-not-answering`、`END abort=1`、`over_*` 行が出ない。これ自体が結果（NetGD に使う材料）なので、そのまま送る。
- 固まった場合は、そこまでの記録が有効（書き出しは各行ごと）。

### 送り返すもの

1. `H:\OpenGDEmu\tools\gdprobe\host\gdprobe_stock_seq00027.txt`（全文）
2. 最新の `H:\DC\tools\dc-bba-tools\logs\auto_*.txt`
3. 固まったか（電源 OFF が要ったか）、TV がどうなっていたか
4. ビルドの警告・エラーがあれば全文

記録を `docs/research/probe-records/stock_<日付>_27_dma-exact.txt` に置くのと LOG の項目は、こちらで行う（ユーザーは置かなくてよい）。

---

## 2. g2netbench_tcpfix（直した KOS の TCP、p1〜p6）

### 何を確かめるか

`tools/kos-patches/net_tcp-rx-fix.patch`（KOS の `net_tcp.c` の受信の直し：順番どおりのセグメントだけ積む、`recv` で窓が半分空いたら窓の更新 ACK、FIN も 1 バイトと数える）を実機の BBA で確かめる。R175・R178：通常の KOS では 64 KiB のフレームがデータ破損、窓より大きいフレームは 4〜6 KB/s だった。直した版で 64 KiB × 1/2 が壊れずに速く通るか。GD-ROM には触れない。

### 準備

- ビルドはプリセットが行う（`-Net -TcpFix`）。先に作るなら：`Set-Location H:\OpenGDEmu\tools\g2bench; .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Net -TcpFix`（`g2netbench_tcpfix.elf`。DreamSDK の `net_tcp.c` を `build-fix\` に写してパッチを当て、リンクマップで `libkallisti.a(net_tcp.o)` が入っていないことを確かめ、違えば止まる）。
- 窓：既定 20480。変えるなら実行前に `$env:G2NET_RCVBUF = 20480`（ビルド時に読まれる）。
- PC：Python 3 が要る（プリセットが `g2n_send.py` を自動で起動する。見つからないと `python not found: start g2n_send.py yourself` と出る）。
- SD：`01`・`02` だけ。Dreamcast：GDmenu → `02`（dcload-ip）。
- 前の実行が残っていないこと（§0）。

### コマンド

```powershell
Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu\tools\g2bench\profiles\g2netbench_tcpfix.ps1 -Ip 192.168.10.2
```

（main のプリセットのフルパスを使う。コピーを `profiles\` に置く方法は使わない。）プリセットの `Exclusive = 'g2bench'` により、g2bench 系は同時に 1 つしか走らない。

### TV に出るもの（README の画面の見方）

行 0 は 1 秒ごとの心拍（`up NNs` が進んでいれば固まっていない）。
1. `dcload type 1  DC 192.168.10.2  PC 192.168.10.x`
2. `netifs registered: 1` → `net_init rc=0 in NNN ms, default dev bba`（`STUCK` なら BBA かリンクの問題）
3. `MAC ...`、`PC MAC ... (NN ms)`（`PC did not answer ARP` なら Dreamcast → PC が通っていない）
4. `fs_dclsocket (exit hand-off): up`、`listening on 192.168.10.2:20039; waiting for the PC sender`
5. PC が繋ぐと `PC connected` → `PUT received: measuring` → 各パスの結果行。終わると 3 秒表示して dcload に戻る。失敗は `FAILED: <理由>` を出して 20 秒後に戻る。

### ログ・出力の場所

- 結果：`H:\OpenGDEmu\tools\g2bench\host\g2net_NNN.txt`（PC の送り手が LINE のたびに追記。`pc_partial=1` のままなら途中で止まった記録）
- 送り手のログ：`H:\OpenGDEmu\tools\g2bench\host\g2n_send_last.log`（と `.err`）
- `dc_auto` のログ：`H:\DC\tools\dc-bba-tools\logs\auto_*.txt`
- 終了時に `PostRun` が送り手のログと最新の `g2net_NNN.txt` を表示する。

所要は約 2 分（README の見積もり）。

### 合否（見込み）

- 合格：パス p1〜p6（16 KiB × 1、64 KiB × 1、64 KiB × 2、32 KiB × 2 など）が `status=ok`、検査ありのパスで `pN_pc_crc_ok=1`、全体で `pc_all_ok=1`、接続が切れない。パスごとに `pN_tcp_ooo_dropped`・`pN_tcp_trimmed`・`pN_tcp_wnd_updates` が出る（直した版が動いた印）。64 KiB のパスが R178 の 4〜6 KB/s より大幅に速い。
- 不合格：`bad_frame_crc`、`timeout`、`time_limit` で数 KB/s、接続が切れる（`pN_conn_dead`）、`pc_all_ok` が 0。
- 速度の基準：R178 の 16 KiB は検査あり 1,566 KB/s、なし 2,133 KB/s。目標値は `docs/research/g2-transport.md`（**要確認**：合格の数値の線はそこにあるか、この実行では決めていない）。

### 送り返すもの

`g2net_NNN.txt`（最新）、`g2n_send_last.log`、`auto_*.txt`、TV で最後に出ていた行（止まったとき）。

---

## 3. g2netbench と g2x（UDP の独自転送）

### 何を確かめるか

`g2netbench.elf`（通常の KOS の TCP）は、TCP の p1・p2（16 KiB の基準）、UDP の連続 u1〜u3（R178 と同じ）、**g2x の x1〜x6**（UDP 20040、1,400 B のデータグラム、データグラムごとの CRC、窓 64、選択 ACK、ペース配分）を実行する。g2x が UDP の取りこぼし（R178：毎秒 2,500 個で 17 % 落ちた）を再送で埋めて、壊れずにどれだけ出るか。

### 準備

- ビルドはプリセットが行う（`-Net`）。先に作るなら：`Set-Location H:\OpenGDEmu\tools\g2bench; .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Net`。
- **PC のファイアウォール**：g2x は Dreamcast → PC の UDP（ACK）が要る。結果に `xN_pc_reason=no_ack` が出たら、Python（`python.exe` / `py.exe`）の受信を許可して再実行（README）。**要確認**：PC の NIC が 192.168.10.1/24 のプロファイル（プライベート／公開）で許可されているか。
- SD・PC・Dreamcast の状態は §2 と同じ。

### コマンド

```powershell
Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu\tools\g2bench\profiles\g2netbench.ps1 -Ip 192.168.10.2
```

### TV に出るもの

§2 と同じ画面（`listening on 192.168.10.2:20039` まで）。g2x のパスでは `LINE` の `G2X go pass=... pps=...` を Dreamcast が PC へ送ってから、PC が UDP 20040 へ流す。README によると `fs_dclsocket` のコンソール出力（`[g2net] ...`）は測定中には出ず、各パスの終わりだけ。

### ログ・出力の場所

§2 と同じ（`H:\OpenGDEmu\tools\g2bench\host\g2net_NNN.txt`、`g2n_send_last.log`、`auto_*.txt`）。キー：`xN_*`（Dreamcast 側）、`xN_pc_*`（PC 側）、`xN_ck=2` のパスは `xN_pc_crc_ok`（PC が全体の CRC を照合）。

所要は約 3 分（g2x の 6 パスは各 20 秒以内）。

### 合否（見込み）

- 合格：g2x の x1〜x6 が完了し、`xN_ck=2` のパスで `xN_pc_crc_ok=1`、`pc_all_ok=1`、`no_ack` が無い。速度（KB/s）と再送・欠けの数が出る。
- 不合格：`xN_pc_reason=no_ack`（ファイアウォールか Dreamcast → PC の UDP が通らない）、CRC 不一致、time_limit。
- 注：R178 の UDP バースト（u1〜u3）も同じ実行に入るので、取りこぼしが前回と同程度か比べられる（毎秒 2,500 個で 17 %、毎秒 1,200 個で 0 %）。

### 送り返すもの

§2 と同じ。`no_ack` が出たらファイアウォールの設定（許可したか）も。

---

## 4. netgd-t2（`T2_NO_DRIVE=1`、ネットだけ）

### 何を確かめるか

NetGD が載る前に、netgd-t2 の KOS 側のうち**ネット部分**だけを実機で確かめる：BBA の手動の立ち上げ（DHCP なし）、20039 での待ち受け、PC の `netgd-send` との HELLO・NGTP のフレーム、書き込みの拒否。`T2_NO_DRIVE=1` のビルドは**ドライブに 1 パケットも送らない**（`tools/netgd-t2/README_ja.md`）ので、純正 GDEMU でも固まらない。書き込みはすべて断る。

### 準備

ビルド（DreamSDK、合言葉を固定する）：

```powershell
Set-Location H:\OpenGDEmu\tools\netgd-t2; .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Make 'T2_NO_DRIVE=1 T2_CODE=1234'
```

（`build.ps1` の例にこの書き方がある。`netgd-t2.elf` ができる。`-TcpFix` を足すと `netgd-t2-tcpfix.elf` もできる。IP は既定 192.168.10.2 で、dcload-ip から起動するときはその IP が使われる。）

- `netgd-t2` には `dc_auto` のプリセットが無いので、`-Elf` と `-NoBuild` で送る（`dc_auto.ps1` のパラメータにある）。PowerShell を 2 つ使う（A：dc_auto、B：PC の送り手）。
- SD：`01`・`02`。Dreamcast：GDmenu → `02`。

### コマンド

窓 A（Dreamcast に送る）：

```powershell
Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Elf H:\OpenGDEmu\tools\netgd-t2\netgd-t2.elf -NoBuild -Ip 192.168.10.2
```

Dreamcast の画面に待ち受けが出てから、窓 B（PC）：

```powershell
py -3 H:\OpenGDEmu\tools\netgd-send\netgd-send.py remote statfs --host 192.168.10.2 --code 1234
```

次にセッションを閉じて終わらせる：

```powershell
py -3 H:\OpenGDEmu\tools\netgd-send\netgd-send.py remote finish --none --host 192.168.10.2 --code 1234
```

（`remote` のサブコマンドと `--host`・`--code` は `tools/netgd-send/README_ja.md` にある。`T2_NO_DRIVE` のホスト試験は `put` と `statfs` が `EFW` で断られ、そのあと `finish` を送る流れ。`finish --none` の実機での終了の形は **要確認**。）

### TV に出るもの

`tools/netgd-t2/src/t2_app.c` の画面（53 × 20）から：
- 1 行目：`netgd-t2 <版>  SD write over NGTP (T2)`、右端に `stock tcp`（tcpfix ビルドは `tcpfix`）
- `IP 192.168.10.2  port 20039  code 1234`
- `WANT 16384 B x 1  rcvbuf 20480`（見込み。tcpfix は 64 KiB × 2）
- 状態行：BBA の立ち上げの各段、`waiting for the PC` など
- `drive: not used (T2_NO_DRIVE build): no writes`
- PC が繋ぐと `client <名前>`、最下行 `B: close the session and leave`。B で終了できる。
- `dc-tool-ip`（窓 A）に最初の行 `NGT2: ... code NNNN`。BBA を KOS が取った後はコンソール出力が途切れる（`g2netbench` と同じ手順）。

BBA なし・`net_init` が返らないときは `FAILED: no network` を出して終了（`README_ja.md`）。

### ログ・出力の場所

- `dc_auto` のログ：`H:\DC\tools\dc-bba-tools\logs\auto_*.txt`
- PC 側：窓 B のコンソール出力（`NGD:` の行がそのまま出る）。ファイルは作られない（**要確認**）。

### 合否（見込み）

- 合格：TV に IP・ポート・合言葉が出る、`remote statfs` が TCP で接続して HELLO が通り、`NGD:ERROR session code=EFW`（書き込みを断る）が返る。ドライブには何も送られない。`remote finish` のあと dcload に戻る（終了コード 0）。
- 不合格：`FAILED: no network`、`PC did not answer ARP`、接続できない（PC のファイアウォール：TCP 20039）、`EAUTH`（合言葉違い。3 回間違えると合言葉が作り直される）。
- 見込みの注意：`statfs` は書き込みの関門（`t2_gate`）を通るため、`T2_NO_DRIVE` では `EFW` で断られるのが**正常**。

### 送り返すもの

TV の写真（待ち受けの画面）、窓 B の出力、`auto_*.txt`、`FAILED` が出たらその理由の行。

---

## 5. netgd-menu 初回確認（純正 GDEMU 上）

### 先に：純正で確かめられること・確かめられないこと

`tools/netgd-menu/src/nm_ui.c` を読んだところ、メニューは起動すると最初に `52 00`（GDEMU_VERSION）を送り、**NetGD の 5.20.0 と答えたときだけ**一覧・SD の読み取りへ進む。純正（5.20.5）では `NM_S_NOFW` の画面（`NetGD firmware not found` / `GDEMU 5.20.5 is not NetGD: no NetGD command sent.` / `Nothing was read from the SD card.`）に入り、START で BIOS に戻るだけ。**この状態では Y（取り込み画面）に入れない**（`NM_S_NOFW` は START だけを見る）。

したがって R187 の確認項目 9 のうち、**「Y をもう一度押しても落ちない」「取り込み画面の出入りで空きメモリが減らない」は純正では実行できない**（NetGD のファームが載って一覧が出てから）。その 2 つはこのランブックの対象外にせざるを得ない。純正で確かめられるのは次の範囲。

| 確かめること | 純正で可 | 根拠 |
|---|---|---|
| H1 起動（`INIT_CDROM` を外した KOS アプリが動くか） | 可 | ELF を dcload-ip から |
| 画面：53 桁の収まり、PAL・VGA（H10） | 可 | NOFW の画面を使う |
| 純正が受け取るのは `52 00` だけ（`E0` を送らない） | 可（ログが無いので間接的） | `fw_probe_netgd` の仕様。ファーム側のログは純正に無い |
| NOFW の文言、START で BIOS に戻る | 可 | `nm_ui.c` |
| R187 の項目 9（Y、No link、再押し、メモリ） | **不可** | NOFW で止まる |
| 一覧・起動（H2・H3・H4・H5・H8） | 不可 | NetGD が要る |

**要確認**：純正のまま Y の経路を通す手段（デバッグ用のビルドなど）はソースに見つからなかった。必要なら、`nm_ui.c` に純正用の迂回を足す別の作業になる（この依頼の範囲外）。「空きメモリ」を画面で確かめる表示も、`plat_kos.c`・`main_kos.c` に見つからなかった（**要確認**：どう測る計画か）。

### 準備

ビルド（DreamSDK）：

```powershell
Set-Location H:\OpenGDEmu\tools\netgd-menu; .\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools
```

- `netgd-menu.elf` ができる（通常の KOS の TCP。`-TcpFix` で `netgd-menu-tcpfix.elf`）。
- **dcload-ip から ELF を送って動かす**（SD の `01` を書き換えない）。`.cdi` を `01` に入れる方法は純正の GDmenu を置き換えてしまうので、捨てても困らない SD でだけ（`make cdi` は `mkisofs`・`cdi4dc` が DreamSDK にあるか **要確認**、README 記載）。
- SD：`01`・`02`。Dreamcast：GDmenu → `02`。

### コマンド

```powershell
Set-Location H:\DC\tools\dc-bba-tools; powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\dc_auto.ps1 -Elf H:\OpenGDEmu\tools\netgd-menu\netgd-menu.elf -NoBuild -Ip 192.168.10.2
```

### TV に出るもの

1. 「Looking for the NetGD firmware ...」（`nm_ui.c` の起動の段）
2. NOFW の画面：`NetGD firmware not found`、`GDEMU 5.20.5 is not NetGD: no NetGD command sent.`、`This menu works with the NetGD firmware only (a stock GDEMU is not supported).`、`Nothing was read from the SD card.`（5.20.5 はプローブ記録 `stock_20261122_08_identify.txt` のとおり。別の版なら数字が変わる）
3. 右上のファームの版の表示は一覧の画面なので、NOFW では出ない（見込み）。

START を押す → BIOS メニューへ（確認なしの `nm_plat_handoff`）。**dcload から起動したときの BIOS への引き渡しの挙動は要確認**。引き渡したあとは Dreamcast が dcload でなくなるので、PC 側の `dc-tool-ip` が残る（stale）。§0 の手順（`-Stop`、電源を入れ直し）で片付ける。

### ログ・出力の場所

- `dc_auto` のログ：`H:\DC\tools\dc-bba-tools\logs\auto_*.txt`（メニューは KOS が BBA を取る前なら dcload のコンソールに出る行があるかもしれないが、**要確認**）
- 画面は TV だけ。写真か、見たままの文言を送る。

### 合否（見込み）

- 合格：起動する（黒画面や固まりがない）、53 桁が収まる（PAL・VGA でも）、NOFW の文言が上のとおり、START で BIOS に戻る、純正が固まらない。
- 不合格：起動しない・黒いまま（H1 の失敗：KOS 側のビルドは CI のみで、実機は未確認）、文言が違う、文字が欠ける（`plat_kos.c` の `X0`・`Y0` を調整する）、START で戻らない、純正が BSY で固まる（電源 OFF。そのときは `52 00` の送り方に問題があるので、どこまで進んだか教えて）。

### 送り返すもの

TV の写真（NOFW の画面、PAL・VGA を試したならその両方）、ビルドの警告・エラー全文、`auto_*.txt`、START のあとの様子（BIOS に戻ったか、`dc-tool-ip` が残ったか）。

### 項目 9（R187）の扱い

純正では実行できないので、**NetGD の MCU が使えるようになってから**行う。内容（`tools/netgd-menu/README_ja.md` の初回確認リスト 9）：BBA なしで Y → `BBA not found`、ケーブルを抜いたまま Y → 約 10 秒で `No link: check the cable and restart the console`、**もう一度 Y を押しても落ちない**（`net_init` を再び呼ばない。KOS 2.3 の `bba_if_start` は失敗後も rx スレッドを残し、2 回目の `net_init` が `assert(bba_rx_thread == NULL)` で落ちるため。R187）、取り込み画面を何度も出入りしても空きメモリが減らない（listen のソケットを閉じずに使い回す）。いずれも実機は未確認。

---

## 付録：要確認の一覧

1. gdprobe：TV の表示（描画しない見込みで、dcload-ip の画面のまま）。25c の所要時間。7 セクタ exact が 1 セクタと同じく終わるか（推測）。
2. g2netbench_tcpfix：合格の速度の線（`docs/research/g2-transport.md` に数値の線があるか）。
3. g2netbench（g2x）：PC のファイアウォールのプロファイル。
4. netgd-t2：`remote finish --none` の実機での終了の形、PC 側のファイル出力の有無。
5. netgd-menu：純正で Y に入る手段、空きメモリの測り方、dcload から BIOS への引き渡し、dcload のコンソールの出力、`make cdi` のツール（`mkisofs`・`cdi4dc`）。
6. 共通：`Exclusive` のプリセット（g2bench 系と gdprobe 系）が、`dc_auto` を別の窓から 2 つ起動したときに互いを待つか置き換えるか（`dc_auto.ps1` の `Stop-OtherRuns` は「前のテストを止めて置き換える」と読めるが、実機で確かめていない）。
