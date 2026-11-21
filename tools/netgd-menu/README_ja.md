# netgd-menu（NetGD 専用メニュー）

SD カードのフォルダ `01` に置く Dreamcast 用メニュー。NetGD のファームにだけ対応する。設計は `docs/design/menu-app.md`。

> **重要：実機では一度も動かしていない。**
> ホスト（PC の gcc）での試験だけが済んでいる（下の「テスト」）。KOS 用の部分（`src/main_kos.c` `src/plat_kos.c` `src/net_kos.c`、それと `tools/netgd-recv` の `dev_kos.c` `fwxfer_kos.c`）は **この PC にツールチェーンが無く、コンパイルは CI（`.github/workflows/kos.yaml`、KOS 2.3.0）だけ。実機では一度も動かしていない**。最初の実機検証では、下の「初回確認リスト」を上から順に確かめること。
> 純正 GDEMU には対応しない（`0xE0` VERSION が `OGDM` を返さなければ、SD に触らず案内だけ出して BIOS に戻る）。

## 何をするか

1. SD カードのルートを **メニュー自身が読み**（セカンダリ IDE ディスク + FatFs、読み取り専用）、番号フォルダ（`02`、`03`、…）と名前フォルダ（`netgd-send --name` で書いたもの）を一覧にする。フォルダ `01`（このメニュー）は出さない。
2. 十字キーで選び、**A** で起動。`0x52`（番号指定）または `E0 05`/`E0 02`（名前指定）でファームにマウントさせ、READY を待ってから BIOS に戻る。BIOS の画面で「プレイ」を選ぶ（既定）。情報画面（X）で「本体を再起動する」方式に切り替えられる。
3. タイトルは先に見つかったもの：`GAME.CFG` の `title`、`NAME.TXT` の 1 行目、GDI の高密度トラックの IP.BIN、フォルダ名。ASCII 以外は `?`。

| ボタン | 一覧 | 取り込み（手順 6 以降） |
|--------|------|-------------------------|
| 上下（アナログも） | 1 件移動（押し続けで連続） | — |
| 左右、L/R | 1 ページ（15 件） | — |
| A | 起動 | 確認画面で許可（1 秒押し続け） |
| X | 情報画面 | — |
| Y | 取り込み画面へ | — |
| START | BIOS メニューへ（確認あり） | — |

## ビルド

DreamSDK のシェル（`KOS_BASE` 設定済み、KOS は 2.3.0 のユーザーフォーク）：

```
cd tools/netgd-menu
make                 # netgd-menu.elf
make cdi             # netgd-menu.cdi（SD のフォルダ 01 用。CI では作らない）
```

PowerShell からは `.\build.ps1 [-Debug] [-Cdi] -BbaTools <Dreamcast-Dev-Tools-for-BBA のフォルダ>`。

- オブジェクトは `build/kos/` に出る（`tools/netgd-recv/src/*.c` と FatFs のオブジェクトもここ。netgd-recv 側のビルドには触らない）。
- `make cdi` が使うツール（`sh-elf-objcopy`、`scramble`、`makeip`、`mkisofs`、`cdi4dc`）は変数で差し替えられる（`MKISOFS=... make cdi`）。**`mkisofs`・`cdi4dc` が DreamSDK に入っているかは未確認**（無ければ `mkdcdisc` に替える）。`ip.txt` の書式も実機の DreamSDK の `makeip` で確かめること。
- `make NM_HANDOFF_DEFAULT=1`：既定の渡し方を再起動にする（0 = BIOS メニュー）。`make NM_SHOW_SLOT1=1`：フォルダ `01` も一覧に出す。

開発中の実行：`dc-tool-ip -t <IP> -x netgd-menu.elf`（一覧と起動だけ。dcload-ip が BBA を使っているので、取り込みは無効：Y を押すと「Started from dcload: use netgd-recv」と出る）。

## SD カードへの置き方

フォルダ `01` に `netgd-menu.cdi` を入れる（他の `.gdi`／`.cdi` は置かない）。ゲームは `02`、`03`、…（2〜4 桁の数字）に置く。`GDEMU.INI` の `reset_goto=1` は、再起動方式を使うかどうか（H3）が分かるまで、渡し方を BIOS（既定）にしたときだけにする。

## テスト（この PC でできるもの）

```
python tools/netgd-menu/test/run_tests.py            # ホストでのメニュー中核の試験（要 gcc、40 件。取り込みは 127.0.0.1 のソケット）
python tools/netgd-recv/test/run_tests.py            # 受信コアの試験（既存 + 軽いマウントとメニュー用コマンド）
```

`menu_host`（`test/menu_host.c`）はメニューの中核を仮想時計で動かす。`menu_host ui <img> <sectors> --script "scan D A t:2000 dump"` のように、ボタンの列を与えて画面とファームへのパケットを見られる。カードのイメージは `tools/netgd-recv` の `test_host mkimage`／`addfile`／`mkdirs` で作る（`run_tests.py` が両方をビルドする）。

実装したもの：一覧・走査・タイトル・起動（0x52／名前）・READY 待ち・情報画面・BIOS への渡し、BBA からの取り込み（NGTP 版 1）。**すべて実機未検証。** 無いもの：UDP の探索、ファーム更新の統合、箱絵（設計の手順 9〜12、任意）。

## BBA からの取り込み（実機未検証）

1. 一覧で **Y**。メニューが BBA を起こし（`net_init`。BBA が無ければ「BBA not found」）、IP アドレス・ポート 20039・4 桁の合言葉を出して待つ。この間は SD カードの読み取り専用マウントを外している（書き込みセッションと同時には持たない）。
2. PC：`python tools/netgd-send/netgd-send.py push game.gdi --host <IP> --code <合言葉> --slot auto`（詳しくは `tools/netgd-send/README_ja.md`）。最初の要求でファームの書き込みセッションが開く（メニューのイメージは排出される。メニューは RAM 上で動き続ける）。
3. 進捗は本体の画面と PC の両方に出る（同じ `NGD:` 行）。**B**：待機中は取り込み画面を出る、転送中は安全に止める（PC は終了コード 2。同じ `push` で続きから再開）。保護フォルダ（`GDEMU.CFG` の既定・`01`）への上書きは、PC の `--force` と本体で **A を 1 秒押し続ける** の両方が要る（B か 30 秒で拒否）。
4. PC が `FINISH` を送る、B で出る、または要求が 5 分来ないとセッションを閉じ（既定のイメージが戻る）、一覧を読み直す。新しいゲームが出る。閉じられなかったときは「Could not close the session: reset the console」と出て止まる（リセットでファームが戻す）。

構成：`src/ngtp.h`・`ngtp_frame.c`（フレーム）、`ngtp_srv.c`（サーバ。受信コア `tools/netgd-recv/src/recv_core.c` を変更せず呼ぶ）、`src/nm_net.h`（ネットワークの境界。KOS は `net_kos.c`、ホスト試験は `test/net_host.c` の 127.0.0.1 のソケット）。KOS のネットワーク（`net_init`、ソケット）の API 名は KOS 2.3.0 のヘッダで確かめていない：`net_kos.c` の `KOS-API-UNVERIFIED` を参照。

## 初回確認リスト（実機、上から順に）

最初は**捨てても困らない SD カード**で。

1. 起動：`netgd-menu.cdi` を SD のフォルダ `01` に入れて電源を入れる。BIOS からメニューが起動するか（H1：`INIT_CDROM` を外した KOS アプリがイメージから起動でき、`fw_xfer`（生 ATAPI）と `g1ata` が両方動くか）。
2. 画面：53 桁が収まるか。PAL（50 Hz）・VGA でも（H10：`plat_kos.c` の `X0`・`Y0` で余白を調整）。右上に `fw 5.20.0` のようにファームの版が出るか。
3. 一覧：番号フォルダが出るか。タイトルが正しいか。100 本で走査にかかる時間（H5）。
4. 起動：A で `0x52` が通り、READY になり、BIOS に戻って「プレイ」で起動するか（H2：BIOS が新しいディスクを認識するか。UNIT ATTENTION の扱い）。
5. 再起動方式（情報画面で切り替え）：`arch_reboot()` が G1 のハードリセットを出すか。`reset_goto=1` のカードでメニューに戻ってしまわないか（H3、ファームのログ「hard reset」）。
6. イメージをマウントしたまま、セカンダリの読み出しが正しく通るか（H4）。
7. 同じ番号のフォルダが 2 つあるときにファームがどちらを選ぶか（H8）。
8. `reset_goto`・`open_time`・`detect_time` の表示が `GDEMU.INI` と合っているか。

9. 取り込み（H6、H7、H9）：BBA なしで Y を押して「BBA not found」がすぐ出るか。BBA ありで IP が出るか（DHCP の待ち時間、固定 IP が使われるか）。`netgd-send push` が最後まで通り、実効速度はどれくらいか（`WANT`／`DATA` の往復、64 KiB ごと）。セッションを閉じた後に `52 82` がすぐ通るか。`tools/netgd-recv/README_ja.md` の初回確認リスト（DMA・FLUSH・FNV 照合・マウント時間）も同じ経路。

KOS 2.3.0 のヘッダで確かめること（CI で通れば確認済み）：`arch_menu()`／`arch_reboot()`（K1）、`bfont` の色設定（netgd-fwupd が使用）、`maple`（netgd-recv が使用）、ネットワーク：`net_init`・`net_default_dev->ip_addr`・TCP の `listen`／`accept`・`O_NONBLOCK`（K2〜K4、`net_kos.c`）。
