# netgd-t2：単体の T2 受け手（NGTP、KOS の TCP → NetGD の SD）

**実機では一度も動かしていない。NetGD もまだ実機に無い。** ホスト試験（模擬ディスク・模擬ファーム・127.0.0.1）と DreamSDK でのビルドだけ。

PC の `tools/netgd-send`（`push` / `remote`）が NGTP（`docs/design/menu-app.md` §8）で送るファイルを受け取り、netgd-recv の受信コア（`recv_core.c`：書き込みセッション、FatFs、CRC・FNV、チェックポイント、読み戻し）で SD に書く。書き方は T1（dcload-ip の `/pc/`、`tools/netgd-recv`）とまったく同じで、違うのはデータの来る道だけ。

## 構成

| 部分 | ファイル | 備考 |
|---|---|---|
| 中核（ホストで試験） | `src/t2_app.c/.h` | 状態（ネット開始 → 待ち受け → 応答 → 終了）、画面（53 × 20）、書き込みの関門、待ち時間の上限 |
| NGTP サーバ | `../netgd-menu/src/ngtp_srv.c`・`ngtp_frame.c` | メニューと共用。WANT の大きさと同時数（`ngtp_srv_set_window`）と書き込みの関門（`may_write`）を足した |
| 受信コア | `../netgd-recv/src/*` | 変更なし |
| KOS 層 | `src/main_kos.c`、`../netgd-menu/src/net_kos.c`・`nm_netup.c`・`plat_kos.c` | BBA は手で立ち上げる（INIT_NET・DHCP なし）。ネットワーク層はメニューと共用 |

### WANT の大きさと同時数

| ビルド | 既定 | 理由 |
|---|---|---|
| `make`（KOS の TCP のまま） | 16 KiB × 1 | R178：窓より大きいフレームは止まりかけ、窓を広げると壊れる |
| `make tcpfix`（直した `net_tcp.c` を一緒にリンク） | **当面 16 KiB × 1** | 64 KiB × 2 は g2netbench p4 と同じ形で、実機の Dreamcast が 2 回止まった（原因は未解明。この変更は止まる形を避けるだけで、直してはいない）。p3 の 64 KiB × 1 も 16 KiB より遅かった。`make tcpfix T2_WANT=65536 T2_DEPTH=2` で明示できる。`docs/research/g2-transport.md` §3。`../kos-patches/net_tcp-rx-fix.patch`（fix/g2-transport-review `f02c1518` の版、FIN を順番の検査に含める）をそのまま使う |

`make tcpfix T2_WANT=… T2_DEPTH=…` で変えられる（1〜65536 B、1〜4）。素の TCP のビルドでは無視され、16 KiB × 1 のまま。受信バッファ（＝TCP の窓）は `T2_RCVBUF`（既定 20480、listen の前に設定）。
**B ボタン**：B は「離された」のを見てから数える。実機で B が押されっぱなしに見えるパッド（400/400 サンプル）があったため。パッドなしは「離された」に数えず、抜き差しや別のパッドではやり直し、離されたと見なすのは同じパッドで 4 サンプル以上かつ 120 ms 以上続けて上がっていたとき（`../netgd-recv/src/ng_btnguard.h`）。B が固まったままなら画面にそう出るので、PC から終えるかリセットする。強制の確認は、質問が出てから A をいったん離し、1 秒長押しすると許可になる（質問の前から押されている A、固まった A、全ボタン押下に見える状態では許可されない）。30 秒で拒否になる。

同時数 2 では、受け手は 2 つ目の WANT を出したまま SD に書くので、PC は次の塊をその間に送れる。DATA は WANT の順に返すこと（netgd-send はそうしている）。ファイルが変わる前と DONE の前には、残った WANT の DATA を読み捨てる。

### 書き込みの関門（純正 GDEMU では何も書かない、`E0` も送らない）

最初の STATFS／LIST／PUT／DELETE／VERIFY のときに 1 回だけ、`fw_probe_netgd`（`../netgd-recv/src/fwctl.c`）でドライブを確かめる。

1. **`52 00 00 00 00 00 00 00 00 00 00 00`（GDEMU_VERSION）**：gdprobe が純正 GDEMU 6.0 に送って記録したのと同じ CDB（`docs/research/probe-records/stock_20261122_08_identify.txt`：8 バイト `00 00 05 02 00 05 20 05` = ブートローダ 2.5.0・ファーム 5.20.5）。UNIT ATTENTION のときだけ同じものをもう 1 回。
2. ファームの語が **NetGD の値 5.20.0（`00 00 20 05`、`mcu/netgd/src/ide.rs` の `GDEMU_COMPAT_VERSION`）のときだけ** `E0 03`（VERSION）を 1 回。UNIT ATTENTION でも繰り返さない。`OGDM` で、機能ビットに SESSION と CHECKSUM が揃っていれば書く。
3. それ以外はすべて `NGD:ERROR session code=EFW msg=...` と `DONE EFW`。以後ドライブには何も送らない。

- **純正（記録どおりの応答）が受け取るのは `52 00` の 1 個だけ**（UNIT ATTENTION があれば 2 個）。ホスト試験で、純正の模型への 0xE0 が 0 個であることを確かめている。
- IDENTIFY PACKET と REQ_MODE は使わない。NetGD はわざと純正と同じバイトを返すので、区別できない。
- どのパケットも `fw_xfer_limit_ms` = 3000 ms で打ち切る（`fwxfer_kos.c`。既定の待ちは BSY 30 秒を含めて最大約 39 秒）。確認全体は最悪約 9 秒で、netgd-send の時間切れより短い。
- 残る危険：5.20.0 と名乗る純正のファームがあれば、`E0 03` が 1 個届く（記録は 5.20.5）。
- HELLO・PING・合言葉違いではドライブに触れない。`make T2_NO_DRIVE=1` はドライブに 1 パケットも送らない（ネットだけの確認用。書き込みは全部断る）。
- KOS の cdrom ドライバは使わない（`INIT_CDROM` なし。netgd-recv と同じ）。

### BBA の立ち上げ

メニューと共用のコード（`../netgd-menu/src/net_kos.c`、手順と時間の上限は `nm_netup.c`、ホスト試験は netgd-menu の `menu_host selftest` U11）。`tools/g2bench/main_net.c`（feat/g2-transport）と同じ手順：`eth_init()` → 各 netif に IP を設定（DHCP させない）→ `net_init(0)` をスレッドで 30 秒まで → PC の ARP を 8 秒まで → dcload-ip から起動したときは終了の引き渡しのためだけに `fs_dclsocket`。IP は dcload-ip のもの、それ以外は `T2_IP`（既定 192.168.10.2、`make T2_IP=0xC0A80A02`）。各手順は画面と（KOS が BBA を取るまで）dc-tool-ip に出る。

### 待ち時間の上限・終わり方

| 条件 | 動作 |
|---|---|
| PC が FINISH | セッションを閉じ（指定どおり）、終了（dcload に戻る、終了コード 0） |
| 待機中に B | セッションを閉じて（既定のイメージを戻す）終了 |
| 作業中に B | 次の安全な点で止まる（`NGD:STOP`、チェックポイントは残る。もう一度 `push` で続き） |
| セッションが開いたまま 5 分フレームなし | セッションを閉じる |
| 15 分フレームなし | 全部閉じて終了 |
| フレームの途中 15 秒、WANT に対する DATA 30 秒 | 接続を切る（セッションは残る。同じ PC がつなぎ直して続ける） |
| BBA なし・`net_init` が返らない | `FAILED: no network` を出して終了 |

## ビルド（DreamSDK）

```powershell
cd tools\netgd-t2
.\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools            # netgd-t2.elf
.\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -TcpFix    # + netgd-t2-tcpfix.elf
```

DreamSDK は書き換えない（tcpfix は `$(KOS_BASE)/kernel/net/net_tcp.c` を `build/kos-fix/` に写してパッチを当て、リンクのマップで KOS の `net_tcp.o` が入っていないことを確かめる）。

## 使い方（NetGD が実機に載ってから）

1. dc-tool-ip で `netgd-t2.elf` を起動。画面（と dc-tool-ip の最初の行 `NGT2: ... code NNNN`）に IP・ポート 20039・合言葉。
2. PC：`py -3 tools\netgd-send\netgd-send.py push game.gdi --host <IP> --code <合言葉> --slot auto`

## ホスト試験

```sh
py -3 tools/netgd-t2/test/run_tests.py
```

33 件：
- 窓：16 KiB × 1、64 KiB × 2、半端な WANT（5000 B × 3）、WANT が 2 つ同時に出ていること。
- DATA の検査：短い DATA の後のデータ（ESTATE、書かない）、順番違い、オフセット違い、WANT より長い DATA、長さの欄が大きすぎる（EPROTO）、DATA の後半が届かず FIN が来た場合（EOF をファイルの終わりと取らない）、フレームの CRC 不一致。
- 中止と再開：PC の中止フラグと再開、中止の後の VERIFY が止まらないこと、B での中止、PC の切断とつなぎ直し、電源断と再開、ファイルの途中のチェックポイント（`--ckp 65536`）からの再開、ファイルの CRC 不一致と上書き。
- 書き込みの関門：純正（`52 00` だけ、UA のときは 2 個、E0 なし、3 秒の上限）、`52 00` に応答なし、別の版の GDEMU、E0 03 への UA を繰り返さない、機能ビット不足、`--no-drive`、HELLO・PING でドライブに触れない、合言葉違い。
- その他：待ち時間の上限、B での終了、BBA なし、`netgd-send.py push`（素の TCP と tcpfix）、純正への push が終了コード 1 で何も書かないこと。
