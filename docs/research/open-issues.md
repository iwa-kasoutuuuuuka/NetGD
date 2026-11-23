# 未解決の問題の一覧（open issues）

作成：2026-11-23（`main` `cf63c921` から切った `docs/plan-r188`）。基準：`docs/research/LOG.md` R188 まで、`docs/research/hw-runbook.md`、2026-11-23 の実機の結果（下の「今日の実機の事実」。**LOG にはまだ無い**）。
各項目は現行のコード・ドキュメントで確かめた（確かめ方は「証拠」に書いた）。直った項目は末尾の「外した項目」にまとめた。**新しい測定値は作っていない。** 原因が分かっていないものは「調査中」と書き、推測は書かない。

読み方：
- 重さ：**blocker** = これが解けるまで G1〜G3 のどれかが先へ進めない／**high** = G2・G1 の主要な道筋を止める／**medium** = 先へは進めるが、実機で確かめるまで信用できない／**low** = 小さい、またはまれ。
- 誰が：**エージェント** = ハードウェア無しで進められる／**Dreamcast** = Dreamcast（と BBA、純正 GDEMU または NetGD）の実行が要る／**基板** = 6.0 基板・Pico のユーザーの手作業が要る（Dreamcast は不要）／**ユーザーの判断** = ユーザーだけが決める。
- 「詰まる先」は、その問題が解けるまで進めない項目。

## 先に：全体の止まり方

```
OI-01 6.0 の SWD が応答しない（ロック前提）
  └─ OI-02 ERASE（ユーザーだけが決める。普段の手順として勧めない）
        └─ OI-03 NetGD が実機で一度も動いていない
              ├─ OI-08 実ピン・PS のタイミング（census → ピン探し → 実ピンのビットストリーム）
              ├─ OI-12 WaitHost の速さ
              └─ OI-13 / OI-15 / OI-16 / OI-17 / OI-20 / OI-21 / OI-22 / OI-23（NetGD の実機でしか確かめられない項目）
純正の GDEMU と Dreamcast だけで進められる：OI-05（seq 25c）、OI-06・OI-07（調査中）、OI-09（メニューの初回）
```

## 今日の実機の事実（2026-11-23、純正 GDEMU 6.0 + Dreamcast + BBA。LOG にはまだ無い）

- `g2netbench`（独自の UDP 転送 g2x）：**すべての計測が通った。最高 2,602 KB/s**（記録：`tools/g2bench/host/g2net_002.txt`、`H:\OpenGDEmu` の作業コピー。コミットはされていない）。
- `g2netbench_tcpfix`（直した KOS の TCP、R185）：**p1・p2 は通り、p3（64 KiB のフレーム）の途中でアプリが終わった**。原因は調査中（ブランチ `diag/hw-exit`、main の外）。→ OI-06。
- `netgd-t2`（`T2_NO_DRIVE=1`）：**起動の約 10〜20 秒後に終わり、ポート 20039 は一度も開かなかった**。原因は調査中。→ OI-07。
- 純正の GDEMU に触れる試験（gdprobe seq 25c、netgd-menu の初回）は**まだ走らせていない**（`hw-runbook.md` §1・§5）。

## 一覧（重さの順）

### blocker

| ID | 問題（1 行） | 詰まる先 | 証拠 | 次の手 | 誰が |
|----|-------------|----------|------|--------|------|
| OI-01 | GDEMU 6.0 の SWD に応答が無い（Pico + debugprobe、DP が無応答）。MCU は**ロックされているものとして扱う**。原因は未解決（候補は JTAGSEL・TST、Pico 側。どれも未確認） | 純正ファームのバックアップ、NetGD の書き込み、G1-2・G1-4、`fwbackup dump`、`census`・ピン探しの実機、G2-5・P6 のすべて | LOG R177・R180、判断の記録 2026-11-22、`tools/fwbackup/README_ja.md`、`docs/research/gdemu60-bringup.md`（N1〜N17） | 安い順に残っているのは、Pico を別のターゲットで自己テストする、JTAGSEL・TST の電位を測る（0.5 mm ピッチで難しい）、2 枚目の 6.0 を用意する。SWD を使わない導入の道は無い（`docs/install-routes`、R180）。ユーザーは 11-22 に調査を止めた | 基板（測定・Pico のテスト）／ユーザーの判断（再開するか） |
| OI-02 | ERASE をするかどうか。ERASE は純正ファームと純正の更新の仕組みを**永久に失う**（戻せない）。**ユーザーだけが決める。普段の手順として勧めない**。2 枚目の基板があってから考える（INSTALL.md §6） | OI-03 以降のうち、SWD が要る段階すべて（ERASE 後の SWD 立ち上げ、`census`、ピン探し、NetGD の書き込み） | 判断の記録 2026-11-22（「行うかどうか・いつ行うかはユーザーが決める」）、LOG R180、`gdemu60-bringup.md` | エージェントは ERASE に触れない・手順に入れない。ユーザーが決めた場合だけ `gdemu60-bringup.md` の順番（`census` → ピン探し → 実ピン → NetGD）で進める | ユーザーの判断 |
| OI-03 | **NetGD は実機で一度も動いていない**。「実装済み」はすべて PC 上のビルド・ホストテスト・シミュレーション・純正の記録との予測の比較だけで、G1・G2（NetGD 側）・G3 の実機検証は 0 件 | G1・G2・G3 の達成の判定、P6、OI-12・OI-13・OI-15〜OI-17・OI-20〜OI-23 | PLAN「いまの状態」、LOG R177（「NetGD は今も実機で一度も動いていない」）、PLAN P6 | OI-01・OI-02 が先。それまでは、ホストテストと純正の記録との比較（`netgd-expect`）を増やす以外にできることが無い | Dreamcast＋基板（OI-01・OI-02 のあと） |

### high

| ID | 問題（1 行） | 詰まる先 | 証拠 | 次の手 | 誰が |
|----|-------------|----------|------|--------|------|
| OI-04 | NetGD の DMA 読み込みが、**32 B の倍数でない長さ**（音声 1 セクタ 2,352 B、Form 2 の 2,324 B、`0x10` の生セクタ、奇数セクタ数）を送り切れるか未確認。純正は要求した長さちょうどで終わる（R183）が、`ide.rs` のコメントは「FPGA は 16 ワード（32 B）の塊がそろったときだけ DMARQ を上げる。DMA 読みでステージするデータは 32 B の倍数でないと**末尾が出ていかない**。今の整セクタの経路はすべて 32 の倍数」と書く。2,048 B 以外の長さは倍数でない。**この読み取りはソースを読んだだけで、実行も FPGA シミュレーションの確認もしていない**。`send_data_in` に 32 B への詰め物の処理は見つからなかった（偶数バイトへの詰め物だけ） | 音声トラック・CDI の Form 2・D10 の DMA での読み込み（BIOS・ゲームが DMA で読む場合）、G1 DMA の互換 | LOG R183（「NetGD の FPGA・ファームも、32 の倍数でない長さをちょうどで送り切る必要がある」）、`mcu/netgd/src/ide.rs:477-484`・`4156-4163`、`fpga/amaranth/hw/ide_device.py`（DMARQ の条件）。R183 のあとにこの点を扱った記録は LOG に無い | FPGA のシミュレーション（`ide_device.py` のテスト）で 2,352 B・2,324 B の DMA を流して末尾が出るか確かめ、出なければ詰め物か FPGA の変更を設計する。OI-05（seq 25c）の `over_n`・`over_hex` が、純正がはみ出す 16 B の中身の材料になる | エージェント（シミュレーション・設計）／最終確認は Dreamcast＋NetGD（OI-03 のあと） |
| OI-05 | gdprobe **seq 25c（G1 DMA、長さちょうど）をまだ走らせていない**。純正は seq 25b の 2 回目で固まった前歴（R183）があり、25c も固まる可能性がある（電源スイッチを OFF にしないと戻らない） | OI-04 の材料（純正がはみ出す 16 B の中身・DMA の速さ）、NetGD の DMA の仕様の確定 | `docs/research/hw-runbook.md` §1、`tools/gdprobe/gdprobe.cfg.seq25c.example`（存在を確認）、LOG R182・R183 | ランブックの順（g2netbench_tcpfix → g2netbench → netgd-t2 → netgd-menu → seq 25c を最後）で、ユーザーが Dreamcast で実行。結果の記録・LOG はこちらで置く | Dreamcast |
| OI-06 | `g2netbench_tcpfix`（直した KOS の TCP）で、p1・p2 は通ったが **p3（64 KiB のフレーム）の途中でアプリが終わった**。**調査中**（`diag/hw-exit`）。原因は分かっていない | G2 の「TCP のまま 64 KiB × 2」（D6）の採否、G2 の転送方式の決定（OI-10） | 2026-11-23 の実機（LOG 未記載）、`hw-runbook.md` §2、ブランチ `diag/hw-exit`（main の外）。元の症状：R175・R178（通常の KOS の TCP は 64 KiB でデータが壊れ、窓より大きいフレームは 4〜6 KB/s） | 調査の結果を待つ。出たら LOG に R189 として記録し、PLAN・本書を更新する | 調査中（別のエージェントが調べている）。再実行は Dreamcast |
| OI-07 | `netgd-t2`（`T2_NO_DRIVE=1`）が**起動の約 10〜20 秒後に終わり、ポート 20039 が一度も開かない**。**調査中**。原因は分かっていない。OI-06 との関係は**調べていない** | G2 の Dreamcast 側の受け手の検証（ネットだけ）、メニューの取り込み（同じ `ngtp_srv.c`）の実機確認 | 2026-11-23 の実機（LOG 未記載）、LOG R186（「実機は未確認」）、`tools/netgd-t2/`、`tools/netgd-menu/src/ngtp_srv.c` | 調査の結果を待つ。出たら LOG に記録し本書を更新する | 調査中（別のエージェントが調べている）。再実行は Dreamcast |
| OI-08 | 6.0 の**実ピン**が分からない：FPGA コンフィグ（PS）の 5 本は v5.5 の値を仮に使っており未確認、PS のタイミングは非公開、ボタン・LED のピンも不明（D6 で保留）。`census`・ピン探し（`fpga/pinfind`）は**実機で一度も走っていない**（ビルドとゲートレベルのシミュレーションだけ） | 6.0 用のビットストリーム、G1 全体、自己更新の LED 表示 | PLAN G1-3、LOG R162・R169・R171・R180、`fpga/pinfind/FIRST_RUN_ja.md`、`docs/research/gdemu60-bringup.md` | 今できるのは、ロックのまま無電源でできる測定 N1〜N17（`gdemu60-bringup.md`、137 番ピンの近くは避ける）。`census` → ピン探しは OI-01・OI-02 のあと | 基板（測定）／ユーザーの判断（ERASE を伴う手順） |

### medium

| ID | 問題（1 行） | 詰まる先 | 証拠 | 次の手 | 誰が |
|----|-------------|----------|------|--------|------|
| OI-09 | **netgd-menu の初回起動（純正 GDEMU 上）が未実行**。純正（5.20.5）では NOFW の画面で止まるので、確かめられるのは起動（H1）・53 桁の収まり（H10）・文言・START で BIOS に戻ることだけ。Y（取り込み）・一覧・起動（H2〜H5・H8）・R187 の確認項目 9 は**純正では確かめられない** | メニューの KOS 層の実機での正しさ（手順 5〜8 の「実機の 1 回目」）、H1・H10 | `hw-runbook.md` §5、`docs/design/menu-app.md` §13・§14、LOG R187 | ユーザーが Dreamcast で `netgd-menu.elf` を dcload-ip から送る（SD の `01` は書き換えない）。純正のまま Y の経路を通す手段はソースに無い（要確認、ランブック §5） | Dreamcast |
| OI-10 | G2 の転送方式が決まっていない：(A) 直した KOS の TCP（NGTP のまま、64 KiB × 2）／(B) 独自の UDP の g2x。g2x は**実機で通った**（最高 2,602 KB/s）が、`g2bench` の中だけにあり、`netgd-t2`・`netgd-menu` には載っていない。(A) は OI-06 の調査中。素の TCP（16 KiB × 1）は約 2 MB/s（R178、実機で確認済み） | G2 の速度の目標（G2-5）、`netgd-recv`・メニューへの採用 | LOG R178・R185（推奨は (A) を先に）、2026-11-23 の実機（g2x、LOG 未記載）、`docs/research/g2-transport.md`、`tools/g2bench/`（`g2x` は g2bench 内だけと grep で確認） | OI-06 の結果が出てから、(A)・(B) の採否を決める。(B) を採るなら g2x を `ngtp_srv` の側に載せる設計が要る | ユーザーの判断（OI-06 のあと）／エージェント（設計） |
| OI-11 | KOS の `poll()` とソケットのミューテックスの**ロック順の逆転**（潜在）。`poll()` は poll のミューテックスを取ってから、各ソケットの `hndl->poll` で `sock->mutex` を取る（`poll.c:69` → `:94`、TCP 側は直した TCP の `net_tcp_fix.c:1962`）。受信側は `sock->mutex` を持ったまま `__poll_event_trigger` で poll のミューテックスを取る（`net_tcp_fix.c:2739`、ソケットは `:2291` で取得）。**ソースを読んだだけで、実行していない**。直す前の `net_tcp.c` にも同じ呼び出しがあるかは、この点検では見ていない。今のコードで `poll()` を呼ぶのは `tools/g2bench/main_net.c:142` だけで、メニューと netgd-t2 は `poll`・`select` を呼ばない（非ブロックの `accept`・`recv`）ことを grep で確認 | `g2bench` の長時間の計測での固まり、将来 `poll()` を使うコード | `/c/DreamSDK/opt/toolchains/dc/kos/kernel/libc/koslib/poll.c` と、`tools/kos-patches/net_tcp-rx-fix.patch` を当てた `net_tcp.c`（本書の作成時に生成して確認）。OI-06・OI-07 との関係は調べていない | (a) `g2bench` から `poll()` をやめる（短い `recv` の試行にする）か、(b) `__poll_event_trigger` を `sock->mutex` の外で呼ぶ変更を `net_tcp-rx-fix.patch` に足す案を書く。確かめるにはホストで鍵の順の模型を作るか、Dreamcast で負荷をかける | エージェント（案とホストの模型）／Dreamcast（確認） |
| OI-12 | 2E の `wait_host` を手書きの future `WaitHost` にした（R188、2E −1,888 B）。**待ちのループ 1 回あたり数十サイクル遅い分は実機で未計測**で、`ide.rs` を通る**ホストテストが無い**（`ide.rs` に `cfg(test)` が無いことを確認）。振る舞いが同じなのは、Fable の読みと `ci-local`・`test_expect`・純正の記録との監査の結果だけ | 2E の速さの確認（G3-3）、`ide.rs` の変更全般の安全網 | LOG R188、`mcu/netgd/src/ide.rs:4433-4516`（`WaitHost`）、`ide.rs` の `cfg(test)` が 0 件 | `WaitHost` の判定の順・タイムアウトを、グローバルを読まない形で `common/` に切り出してホストテストを書く（`discimg` の P4 と同じ方針）。速さは NetGD の実機で測る（OI-03 のあと） | エージェント（切り出し・テスト）／Dreamcast＋NetGD（速さ） |
| OI-13 | 起動時のリセット（BIOS・KOS）のあと、`detect_time` の間 NOT READY と UA が出ても、**起動できるか**が実機で未確認。NetGD の DEVICE RESET・同じフォルダへの再切替・切替の窓・`CD_READ` の種別検査（D8・D9）・seq 21〜24 の頭の位置の規則なども、純正の記録と**予測で一致しただけ**で NetGD の実機は未確認 | G1・G3 の実機での起動、BIOS・GDmenu・openMenu との互換 | PLAN「いまの状態」（「実機で未確認：起動時のリセット…」）、LOG R161・R164・R170・R179 | OI-03 のあと。純正の記録（`probe-records/`）と NetGD の gdprobe の記録を `compare.py` で比べる（P5.5 と同じ手順） | Dreamcast＋NetGD |
| OI-14 | **6.0 のビットストリームの未確認項目**：Supra のモデルがシリコンどおりか、ゲートレベルのシミュレーションが動かさない範囲（形式的な等価性ではない、R184）、同じ入力でも出力が毎回変わる（LZMA 後 36.7〜40.0 KB）、埋め込みか SD の `NETGD.BIT` かの最終判断は PS の転送時間と実ピンでの再測定の後（R99・判断の記録）、PS で流す時間（約 1.2〜1.7 秒）は見積もりだけ | 6.0 の FPGA が NetGD のコアで動くか（G1）、埋め込みの採否 | LOG R99・R106・R158・R184、`docs/research/ag16k-supra-build.md` §11〜§14、`docs/design/g1-bitstream-storage.md` | ホストで進められるのは、ゲートシミュレーションの範囲を広げること。残りは実ピンと実機（OI-08） | エージェント（範囲の拡大）／基板・Dreamcast（本体） |
| OI-15 | **自己更新（`0xF9`）で起動不能になる窓**：対策 A（型番照合・ベクタ検査など）だけでは、転送中の電源断などで起動不能になる窓が残る。対策 B（8 KB の常駐部＋SD 復旧）は S0〜S2 までホスト側に実装済みで、**S3 以降（常駐部の実機での割り込み・クロック、SD 初期化、ロックと EWP の両立）は未着手**。ロック前提では SWD で戻せない | NetGD 導入後の自己更新の使用、メニューの手順 10（`FWPUSH`）、`netgd-fwupd` の実機 | PLAN「自己更新」節、LOG R46・R54・R58〜R61・R79、`docs/design/fw-bootloader.md`、`docs/design/resident-app-contract.md` | NetGD が実機で動くまで自己更新を使わない。ホストでの追加の確認は可 | Dreamcast＋基板（OI-03 のあと） |
| OI-16 | **P6 の既知の要確認**：上流で報告された IDE バスのビット化け（バスタイミング）、F5 の前提（書き込み中に Dreamcast が RSTn を動かさない、R16）、書き込み中の `FWS=6` の必要性（R59）、`stack-hw` による 2E の最高水位（静的な見積もりの余裕は約 2.2 KB、R131・R133） | G1・G2 の信頼性 | PLAN P6、LOG R16・R59・R127・R131・R133 | 実機で NetGD が動いてから、`netgd-recv`・`gdprobe` の初回確認リストで | Dreamcast＋NetGD（OI-03 のあと） |
| OI-17 | **G3 の性能が NetGD で未計測**：G3 の判定基準は「読み込み時間などの計測値が本家以上」。本家の基準値は取れた（約 2,280 KiB/s など、R117〜R124）が、NetGD の値は無い。G3-4（`sd-fast`、`release-z`）は既定オフで、実機で比べてから決める | G3 の達成の判定、`sd-fast`・`release-z` を既定にするか | PLAN G3-3・G3-4、LOG R72・R75・R98、`docs/research/flash-budget-2e.md` §7 | OI-03 のあと | Dreamcast＋NetGD |

### low

| ID | 問題（1 行） | 詰まる先 | 証拠 | 次の手 | 誰が |
|----|-------------|----------|------|--------|------|
| OI-18 | メニューの BBA まわりに**今は直さない小さい危険**が残る（R187 の 2 回目のレビュー）：(1) 残した listen のソケットに PC がつないで消えると約 28 KiB が残る、(2) 一覧の画面で PC がつなぐと 1〜2 秒後に拒否になる、(3) Y の約 1 秒前につないだ送信は 1 回切れる。さらに、KOS 2.3 の `bba_if_start` はリンクを 10 秒待って失敗すると rx スレッドを残して -1 を返す（メニューは `net_init` を 1 起動で 1 回だけに制限して避けている）。R187 の実機の確認項目 9（Y をもう一度押して落ちない・転送画面の出入りで空きメモリが減らない）も未確認 | なし（まれ）。確認項目 9 は OI-03 のあと | LOG R187、`tools/netgd-menu/src/net_kos.c`（`parked` の説明、`g_net_tried`） | 気になるなら (1)〜(3) を直す案を書く。確認項目 9 は NetGD のファームが載った実機で | エージェント（案）／Dreamcast＋NetGD（確認項目 9） |
| OI-19 | 純正との**残る差**：CDI 04 の Form 2 の 2,324 B（`form2-sectors` は既定オフ、D14）、純正が止まる XEND（R176・R179。NetGD は 5/00/00 で断る方針）、D11（純正が拒む GDI の形を NetGD は受け付けたまま）、D15（バージョン文字列）。XPRE2 の混在は R181 で対応済み | なし。互換の細部（BIOS・ゲームでの影響は未確認） | LOG R151・R174・R176・R179・R181、`docs/research/stock-diff-audit.md` | 影響が出たら直す。D14 を既定にするかは実機で決める | ユーザーの判断（D14 を既定にするか）／Dreamcast＋NetGD |
| OI-20 | **netgd-menu の手順 10（`FWPUSH` でのファーム更新）と手順 12（実機の数字で決めるもの：走査のキャッシュ、直接起動のローダー、CDI のタイトル、T2 の 2 スレッド化、日本語の文言）が未着手**。どちらも「実機の後」。手順 9（UDP の探索）・11（箱絵）はホスト側は実装済み | メニューの完成 | PLAN「ソフト側の残り」、`docs/design/menu-app.md` §13・§14（H5・H6） | 手順 10 は対策 B が実機で動いたあと（OI-15）、手順 12 は実機の数字が出てから別の設計を書く | Dreamcast＋NetGD（OI-03・OI-15 のあと） |
| OI-21 | **G2-2 の性能（F9〜F12）が未着手**（F9 ACMD23、F10 CMD25 の DMAC 化、F11 CMD6 の高速モード、F12 書き込み中の defmt の削減。「計測してから着手を決める」）。SD の活線挿抜・マウント解除と再スキャン、FIFO/DMA の書き込み経路、G3-4 の FIFO の深さ・ATAPI 応答の遅延も未着手 | G2 の書き込み性能の目標（連続書き込み 1 MB/s 以上） | PLAN G2-2・G3-2 の 5・G3-4、`docs/design/bba-sd-write.md` の F9〜F12（173〜179 行） | 実機で計測してから（OI-03 のあと）。F12 は 2E の容量の面でも先に効く | Dreamcast＋NetGD |
| OI-22 | 2E のフラッシュの余裕：`release-z` の `resident,sd-fast` で used 116,336 B・空き 6,544 B（上限 122,880 B、R188）。`release`（既定）では `resident,sd-fast` が入らない（R111）。`release-z` を既定にするかは実機で速度を比べてから | 機能を足すときの余地、既定のプロファイルの決定 | LOG R98・R111・R168・R171・R188、`docs/research/flash-budget-2e.md`、`tools/fwsize/check.py --gate-2e` | 機能を足す前に fwsize のゲートを見る。既定にするかは OI-17 のあと | エージェント（容量の管理）／Dreamcast＋NetGD（速度の比較） |
| OI-23 | 切替の完了から SECNR 0x06 までの時間（純正は 3〜6 ms、NetGD は数 µs で未調整）と、空のフォルダ・媒体なしへの切替で 0x06 を先に出す仮説（純正は 1 ms 以内、実機未確認） | なし（互換の細部） | LOG R171（`fix/switch-complete-first`・`fix/switch-empty-folder`） | OI-13 の実機の比較で見る | Dreamcast＋NetGD |

## 外した項目（確かめた結果、直っていたもの）

- **PLAN の「ピン探しの初回の Supra ビルドは未実施」**：R162（Supra で最後まで通った）と R169・R171（段階 0・1 を RETIMING 修正のあとで作り直し、ゲートシミュレーションも通過）で済んでいる。残るのは実機の実行だけ（OI-08）。
- **`seq 19・20` と `g2bench` が main の外**（PLAN の古い記述）：`feat/gdprobe-seq20`・`feat/g2bench` は main に取り込み済み（`git merge-base --is-ancestor` で確認）。
- **`ack_command` の取りこぼし（A-c の残り）**：R100 で番号つき ack に直した（R103 で rbf も差し替え）。
- **FAD 150 未満を LBA 0 として返す不具合**：R174 で直した。
- **KOS 2.3 の TCP の「64 KiB でデータが壊れる」**：R185 の直した TCP（`tools/kos-patches/net_tcp-rx-fix.patch`）と FIN の穴の修正で、ソースの上では直した。**実機は OI-06 のとおり p3 の途中で終わっているので、直ったとは言えない**（外したのは設計上の穴の部分だけ）。
- **メニューの `net_init` の 2 回目で落ちる（`bba_rx_thread == NULL` の assert）・listen のソケットの 20 KiB のリーク**：R187 で直した（残る小さい危険は OI-18）。
- **`fwbackup check` が NO_TARGET を BAD_CHIP_NAME と誤表示**：`fix/fwbackup-no-target`（`b6ed93fe`）で直し、main に取り込み済み。
- **D-1（トラック 2 本の CDI でセッション 2 が返らない）**：R71・R78・R86 で修正済み。D-4 は変更なしで決着。
- **共有の `git stash` の事故**（R180）：エージェントへの指示で stash を禁止する運用にした（コードの問題ではない）。
- **AGM の `synth_agm` でのメモリ推論が未確認**（PLAN の古い記述）：R99 で Supra 2026.03 により M9K 14 個でビルドできた。
- **2E の空きが 348 B（R100）・5,648 B（R168）・4,880 B（R181）**：R188 で 6,544 B に更新（OI-22）。

## 関連

`docs/PLAN.md`（計画と状態）、`docs/research/LOG.md`（記録）、`docs/research/hw-runbook.md`（実機の実行の手順）、`docs/research/gdemu60-bringup.md`（ERASE したあとの立ち上げと今できる測定）。
