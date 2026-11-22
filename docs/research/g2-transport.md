# G2 の転送路：KOS の TCP を直すか、UDP の独自転送か（H6 の続き）

- 対象：NetGD の取り込み（`docs/design/menu-app.md` §8 の NGTP、D6）で、PC → Dreamcast の受信を速くする方法。
- 前提の実測：R175・R178（`feat/g2bench` の g2netbench）。KOS 2.3（DreamSDK R4 同梱）、BBA、PC は Windows。
- 作ったもの：ブランチ `feat/g2-transport`（`feat/g2bench` から）。**まだ実機では動かしていない**。
- GD-ROM には触れない（`INIT_CDROM` なし、G1・ATA・cdrom の呼び出しなし。g2netbench と同じ）。DreamSDK は書き換えない。

## 0. 結論（先に）

1. **製品の経路は (A) KOS の TCP を直す、を先に採る。** 原因は 2 つのはっきりした不具合（順番違いのセグメントを順番どおりとして積む、`recv` で窓が開いても ACK を送らない）で、直しは `net_tcp.c` の約 40 行。NGTP（TCP、`WANT`／`DATA`、netgd-send）をそのまま使え、D6 の 64 KiB の `DATA` と先読み（`WANT` 2 つ）が使えるようになる。DreamSDK には手を入れず、**直した `net_tcp.c` をアプリに一緒にリンクして KOS の同名の部品を置き換えられる**ことを確かめた（§3）。
2. **(B) UDP の独自転送（g2x）は計測用に実装した**（g2netbench の新しいパス、PC 側の送り手、ホストの模擬回線テスト）。BBA の受信リング（16 KiB）を溢れさせないペース配分と、窓（受け手のバッファ）を切り離せるのが TCP に無い利点。次の実機で (A) より **2 割以上**速く、かつ取りこぼしからの回復が安定していたら、取り込みのデータ部分だけを g2x に移す（制御は NGTP の TCP のまま、§4.2）。
3. どちらにしても上限は **Dreamcast が 1 パケットを受け取る手間**（BBA のリングからの G2 越しの読み出し、KOS の IP・UDP／TCP の処理、ACK の送信）で決まり、R178 の UDP から見て **約 2.8〜2.9 MB/s** 前後と見込む（推定）。今の 2.1 MB/s からの伸びは大きくても 3〜4 割。`g2-perf-plan.md` の中心の見積もりでは、ネットが 2.1 → 2.6〜2.9 MB/s になっても全体は約 +8〜12 %（SD 書き込みと読み戻しが残る）。**複雑さの小さい (A) で十分な可能性が高い**ので、(B) は測ってから決める。

## 1. 分かっていること（R175・R178）

| 条件 | 結果 |
|---|---|
| TCP、16 KiB `DATA`、窓 20 KiB、`WANT` 1 つずつ | チェックサムあり 1,566 KB/s、なし **2,133 KB/s**（壊れない） |
| TCP、窓 65,535 | 64 KiB の 2 枚目のフレームで CRC 不一致（データが壊れる） |
| TCP、32〜64 KiB `DATA`（窓 20 KiB） | **4〜6 KB/s**（20 秒で 2〜3 フレーム）。壊れはしない |
| UDP、1,400 B × 4,000 個 | 毎秒 1,200 個で欠けなし（1,641 KB/s）、毎秒 2,500 個で 17 % 欠け（届いた分で 2,830 KB/s） |

KOS のソース（`kernel/net/net_tcp.c`、`kernel/arch/dreamcast/hardware/network/broadband_adapter.c`）で読んだこと：

- **受信リング**：BBA（RTL8139）の受信バッファは `RX_BUFFER_SHIFT 1` = **16 KiB**。1,514 B のフレームなら約 11 個で一杯になる。BBA のスレッド（優先度 1）が G2 越しに取り出す。PC は 100 Mbit/s で約 8,200 フレーム/秒を送れるが、Dreamcast が捌けるのは R178 から約 2,000〜2,900 個/秒なので、**一度に続けて来るのが約 14 個を超えると溢れる**（R178 の窓 20 KiB = 14 セグメントがぎりぎり無損失だったのと合う）。
- **不具合 1（データが壊れる）**：`process_pkt` は、セグメントの先頭が窓の中にあれば `seq == rcv.nxt` を確かめずに受信バッファの末尾へ足す。リングで 1 個落ちると、その次のセグメントが「続き」として積まれ、ストリームがずれる（R175 の CRC 不一致）。
- **不具合 2（止まる）**：`net_tcp_recvfrom` は取り出した分だけ `rcv.wnd` を増やすが、ACK を送らない。フレームが窓より大きいと、窓が 0 になった PC は Windows の持続タイマー（ゼロウィンドウの確認）を待つ。小さいフレームで止まらないのは、Dreamcast が次に出す `WANT` のセグメントが窓の知らせを兼ねるから。
- UDP は受信のたびに `malloc` してキューに積む（上限なし）。チェックサムは KOS が確かめる。取りこぼしはリング（BBA）で起きている。

## 2. 2 つの案の比較

| 観点 | (A) KOS の TCP を直す | (B) UDP の独自転送（g2x） |
|---|---|---|
| 変える量（Dreamcast） | `net_tcp.c` に約 40 行（パッチ）。アプリ側は NGTP のまま、`DATA` を 64 KiB・`WANT` を 2 つに | 新しい受け手（`g2x_core.c` 約 250 行）と、NGTP との組み合わせ（§4.2） |
| 変える量（PC） | なし（netgd-send の NGTP のまま） | 送り手（`g2x_send.py` 約 300 行）、ペース配分 |
| 取りこぼしへの強さ | 落ちたら PC の高速再送（重複 ACK 3 つ）。**SACK なし**なので 1 つの窓で複数落ちると往復ごとに 1 個ずつ。窓の末尾で落ちると重複 ACK が来ず、**Windows の再送タイマー（最小 300 ms 前後）待ち** | 選択 ACK（63 ビット）で穴はすぐ分かり、1 往復でまとめて再送。末尾の欠けは 30 ms で再送 |
| リングを溢れさせない工夫 | できない（PC の TCP は窓いっぱいを回線速度で送る）。**窓 ≈ 1 回の連続送信量**なので、窓は 20 KiB 前後に抑えるしかない | ペース配分（毎秒 N 個、追いつき送信は 3 個まで）で連続を短くし、**窓（88 KB、増やせる）と連続の長さを切り離せる** |
| ACK の数（Dreamcast の送信） | 受け取ったセグメントごとに 1 個（KOS は遅延 ACK をしない） | 16 個ごと + 穴を見つけたとき（1 ms に 1 回まで）+ 4 ms の無通信 |
| 1 個あたりの検査 | TCP のチェックサム（KOS）+ NGTP のフレーム CRC | UDP のチェックサム（KOS）+ データグラムごとの CRC-32（任意、フレーム CRC の代わり） |
| 再開・オフセット指定 | NGTP の `WANT` そのまま | `WANT` に「g2x で送れ」を足す（§4.2） |
| KOS の版を上げたとき | パッチを当て直す（ビルドが自動で当てる。当たらなければビルドが止まる）。上流へ出せる | 影響なし（UDP の公開 API だけ） |
| リスク | Windows の TCP の振る舞い（輻輳制御・再送タイマー）は変えられない | 独自プロトコルの保守。ファイアウォールが ACK を落とすと進まない |
| 見込み（推定） | 約 2.3〜2.6 MB/s（フレームごとの往復待ちが消える分） | 約 2.5〜2.9 MB/s（ACK の送信が減る分、ただしデータグラム CRC で CPU を使う） |

**どちらを先に**：(A)。理由は (1) 今の 2 つの問題（壊れる、止まる）はどちらも KOS の不具合で、直せば NGTP の設計（D6）が当初の形で使える、(2) PC 側も netgd-send も変えなくてよい、(3) (B) の伸びは見込みで 1〜2 割、全体（SD 書き込み・読み戻しを含む）では数 % しか効かない。(B) は (A) の結果が悪いときの次の手で、今回はその判断ができるよう計測用に作った。

## 3. (A) KOS の TCP の直し

パッチ：`tools/kos-patches/net_tcp-rx-fix.patch`（KOS 2.3.0 の `kernel/net/net_tcp.c` に対する差分、改行は LF）。

1. **順番違いを積まない**：データ付きのセグメントは `seq == rcv.nxt` のときだけ受信バッファへ。先へ飛んだものは捨てて重複 ACK を返す（PC の高速再送が穴を埋める）。`rcv.nxt` より前から始まり窓へ入り込む再送は、持っている部分を切り捨てて残りだけ積む（受け入れの条件を RFC 793 のとおり「窓と重なれば可」に広げた）。順番違いのセグメントの FIN は扱わない。
2. **窓が開いたら知らせる**：`recv` が取り出して、窓が受信バッファの半分を下から上へ越えたら ACK（窓の更新）を 1 個送る（RFC 1122 4.2.3.3 の受け手側の SWS 回避）。接続中（ESTABLISHED・FIN_WAIT_1/2）だけ。
3. 数え：`net_tcp_fix_ooo_dropped`（捨てた順番違い）、`net_tcp_fix_trimmed`（切り捨てた再送）、`net_tcp_fix_wnd_updates`（送った窓の更新）。g2netbench_tcpfix がパスごとの増分を報告する。

**DreamSDK を変えずに使う方法（確認済み）**：KOS の `net_tcp.o` の外向きの名前は `net_tcp_init` と `net_tcp_shutdown` の 2 つだけ（`sh-elf-nm` で確認）。アプリのオブジェクトはリンクの命令で `-lkallisti` より前に来るので、同じ 2 つを定義した直した版をアプリに入れると、リンカは `libkallisti.a` の `net_tcp.o` を取り出さない（未解決の名前が無いので）。`tools/g2bench/Makefile` の `net-tcpfix` は：

1. `$(KOS_BASE)/kernel/net/net_tcp.c` を `build-fix/` へ写し（CR を落とす）、パッチを当てる（`patch`、当たらなければ止まる）。
2. `-I$(KOS_BASE)/kernel/net`（`net_ipv4.h` などの内部ヘッダ、読むだけ）で `net_tcp_fix.o` を作る。
3. リンクのマップ（`build-fix/tcpfix.map`）で、`net_tcp_fix_wnd_updates` があり `libkallisti.a(net_tcp.o)` が無いことを確かめる（無ければビルド失敗）。

2026-11-22 のビルドで `tcpfix: patched net_tcp.c linked, libkallisti.a(net_tcp.o) not pulled`、`net_tcp_init` は `net_tcp_fix.o` から（マップで確認）。警告なし。製品（menu-app）でも同じ作りで使える。KOS を上げたらパッチが当たるか確かめ、当たらなければ当て直す。上流（KallistiOS）へ出す価値もある。

**(A) で残る弱さ**：KOS は SACK も遅延 ACK も持たない。窓を 20 KiB より広げると、PC の連続送信がリングを溢れさせ、今度は壊れはしないが再送（とくに窓の末尾での再送タイマー待ち）で遅くなるはず。だから **(A) の推奨の設定は窓 20 KiB のまま、`DATA` 64 KiB、`WANT` 2 つ**（窓の更新 ACK でフレームが窓より大きくても流れ続ける）。窓 32 KiB・64 KiB は「直しが壊れを防ぐか」の確認用（`-RcvBuf` で作り直す）。

## 4. (B) g2x：UDP の独自転送（R7）

コード：`tools/g2bench/g2x_core.c/h`（Dreamcast とホストで共通の C、受け手と送り手の手順）、`tools/g2bench/g2x_send.py`（PC の送り手、同じ手順の Python 版と、テスト用の受け手）。

- **データグラム**：1,400 B（見出し 20 B + データ 1,380 B）。見出し：`"G2X1"`、種別、フラグ（再送・CRC あり）、パス番号、通し番号（データの位置 = 番号 × 1,380）、長さ、CRC-32（見出しの 16 B とデータ、zlib）。1,400 B は R178 の UDP と同じで、IP 断片化しない。
- **ACK**（40 B）：`cum`（ここまで全部受け取った）、`limit`（送ってよい番号の上限 = `cum` + 64）、`cum + 1` から 63 個分の受け取り済みビット、ACK 番号、受け手の数え（受信・重複・不良）。完了（DONE）・打ち切り（STOP、時間切れ）のフラグ。
- **受け手**（Dreamcast）：64 枠（88 KB）の窓。順番どおりに来たものは**写さずにそのまま渡す**、先に来たものだけ枠に置き、穴が埋まったらまとめて渡す。CRC 不良・長さ違いは「落ちた」と同じ扱い（送り手が再送）。ACK は順番どおり 16 個ごと、穴や重複を見たら（1 ms に 1 回まで）、何か未報告のまま 4 ms 経ったら。`limit` があるので KOS の UDP のキュー（上限なし）も 64 個 + 再送分までしか溜まらない。
- **送り手**（PC）：毎秒 N 個のペースで送る（遅れても追いつき送信は 3 個まで：リングは約 11 個）。BBA のリングは FIFO で順番が入れ替わらないので、**「後から送ったものが届いたのに、それより前に送ったものが無い」= 落ちた**として、タイマーを待たずに再送する（RACK に近い考え方）。30 ms 何も確認されなければ、送ったまま未確認のものを全部再送する（末尾の欠け）。速さの自動調整（任意）：前進のある ACK ごとに +3 個/秒、落ちたら ×0.85（20 ms に 1 回まで）、下限・上限あり。
- **制御**：今回の計測では NGTP の TCP の `LINE`（`G2X go pass=.. bytes=.. pps=.. min=.. max=.. crc=..`）で始め、結果も `LINE` で返す。データは UDP 20040。

### 4.1 ホストでの確認（`sh tools/g2bench/tests/run_tests.sh`）

- `tests/test_xport.c`（C）：見出し・ACK の組み立てと検査、受け手の窓（穴、重複、CRC 必須、別のパス）、**模擬回線**：PC →（遅延・無作為な欠け・重複・ビット反転）→ BBA のリング（16 KiB、FIFO、満杯で捨てる）→ Dreamcast（1 個 340 µs で取り出す ≒ 毎秒 2,900 個）→ ACK（遅延・欠け）→ PC。8 MiB ずつ：

| 場合 | 模擬の速さ | 再送 |
|---|---:|---|
| 毎秒 1,200 個、損失なし | 1,618 KB/s | 0 |
| 毎秒 2,500 個、損失なし | 3,369 KB/s | 0 |
| 毎秒 4,000 個（取り出しより速い：リングが溢れる） | 3,903 KB/s | 2,172（全部リングでの欠け、再送タイマー 1 回） |
| 毎秒 2,000 個、欠け 3 %・ACK 欠け 5 %・重複 1 %・ビット反転 0.5 % | 2,589 KB/s | 251（反転 36 個は CRC で捕まえた） |
| 自動調整 800〜4,000 | 3,183 KB/s | 1（2,912 個/秒に落ち着く） |
| 自動調整、欠け 10 %・ACK 欠け 20 % | 987 KB/s | 659（無作為な欠けを混雑と見て下限まで下げる：仕様どおり） |

  どの場合もデータは順番どおり・内容一致・CRC 一致。模擬の Dreamcast の手間（340 µs）は R178 からの推定で、実機の値ではない。

- `tests/test_g2x.py`（Python）：本物の UDP（localhost）で、送り手（`g2x_send.send_pass`）と受け手（`g2x_send.Rx`）の間に欠け・ACK 欠け・重複・連続 13 個の欠け（リングの溢れの代わり）・ビット反転を入れて、内容一致・再送・速さの自動調整・時間切れの STOP・小さい転送を確かめる。C の組み立てと Python の組み立てが同じバイト列になることも確かめる（`xport_vec.bin`）。
- `tests/test_sender.py`：偽の Dreamcast を相手に、NGTP の接続の中で g2x のパスを 1 回（7 個に 1 個を落とす）通し、PC の CRC の照合（`x1_pc_crc_ok`）まで確かめる。

### 4.2 製品に入れるなら（未設計の要点）

- NGTP の `WANT` にフラグ「g2x で送れ」を足し、長さを大きく（たとえばファイルの残り全部）取れるようにする。PC は UDP で流し、Dreamcast の `ng_src_t.read` は g2x の受け手から順番どおりに読む（D6 の「読む側が呼ぶ」形は保てる）。中止・再開は今の `WANT`／チェックポイントのまま。
- データグラムの CRC-32 を NGTP のフレーム CRC の代わりにする（ファイルの CRC・FNV は今の netgd-recv のまま）。
- 窓（枠の数）は SD への書き込みと重ねたいだけ増やせる（リングの心配はペース配分が受け持つ）。

## 5. 次の実機での測り方

### 5.1 ビルド（DreamSDK、済み）

```powershell
cd H:\OpenGDEmu-g2xport\tools\g2bench
.\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Net           # g2netbench.elf：TCP p1・p2（基準）+ UDP の連続 3 回 + g2x 6 パス
.\build.ps1 -BbaTools H:\DC\tools\dc-bba-tools -Net -TcpFix   # g2netbench_tcpfix.elf：直した TCP で 6 パス（窓 20 KiB）
```

### 5.2 実行（1 回ずつ。GDmenu から DCLOAD-IP BBA を起動してから `H:\DC\tools\dc-bba-tools` で）

```powershell
.\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu-g2xport\tools\g2bench\profiles\g2netbench_tcpfix.ps1
.\scripts\dc_auto.ps1 -Preset H:\OpenGDEmu-g2xport\tools\g2bench\profiles\g2netbench.ps1
```

結果は `H:\OpenGDEmu-g2xport\tools\g2bench\host\g2net_NNN.txt`。所要はそれぞれ 2〜3 分。窓を変えた (A) の確認は `$env:G2NET_RCVBUF = 32768`（または 65535）でビルドし直してから同じプリセット（`tcpfix` は窓を `listen_rcvbuf=` に書く）。

### 5.3 パス

| ELF | パス | 中身 |
|---|---|---|
| tcpfix | p1・p2 | 16 KiB・`WANT` 1 つ、チェックサムあり／なし（R178 の 1,566／2,133 KB/s と比べる：直しで遅くならないこと） |
| tcpfix | p3 | 64 KiB・`WANT` 1 つ・チェックサムあり（R178 では 4〜6 KB/s） |
| tcpfix | p4・p5 | 64 KiB・`WANT` 2 つ、チェックサムあり／なし（**(A) の本命**） |
| tcpfix | p6 | 32 KiB・`WANT` 2 つ |
| 通常 | p1・p2、u1〜u3 | R178 と同じ（基準と、UDP の受け取り能力） |
| 通常 | x1・x2 | g2x、毎秒 1,200・2,000 個固定、データグラム CRC あり |
| 通常 | x3・x4・x5 | g2x、自動調整 800〜4,000、検査：CRC あり／なし／CRC + ファイルの CRC・FNV（netgd-recv と同じ仕事、PC が CRC を照合） |
| 通常 | x6 | g2x、毎秒 2,500 個固定（R178 で 17 % 落ちた速さ：回復が効くか） |

### 5.4 見るところ

- **tcpfix の p3〜p6 が MB/s 台になるか**。`p*_tcp_wnd_updates` が増えていれば窓の更新 ACK が働いている。`p*_tcp_ooo_dropped` が 0 でなければリングで落ちて直しが働いた（データは壊れないはず、`p*_pc_crc_ok=1`）。窓 20 KiB でも多いなら、窓を 16 KiB 台へ。
- **p4（64 KiB × 2）の速さ**と、通常版の **x3（自動調整、CRC あり）の速さ**を比べる。x3 が p4 より 2 割以上速く、`x3_pc_resent` が少なく安定なら (B) へ進む。差が小さければ (A) で確定。
- x の `x*_bad`（CRC 不良：0 のはず）、`x*_gaps`（新しい穴の数）、`x*_pc_resent`／`x*_pc_rto`（再送・末尾の再送）、`x*_pc_pps_end`／`x*_pc_pps_avg`（自動調整が落ち着いた速さ。これが Dreamcast の捌ける上限の目安）。
- x5 の `x5_pc_crc_ok=1`（g2x のデータが最初から最後まで正しい）。
- **`x*_pc_reason=no_ack` が並んだら**：Windows のファイアウォールが Dreamcast からの UDP（ACK）を落としている疑い。Python に受信を許可するか、一時的に確認する（UDP の連続 u1〜u3 は PC → Dreamcast だけなので今まで問題にならなかった）。
- 画面は x パスでも 0 行目の心拍（`rx=` が増える）と、各パスの終わりの 1 行。

## 6. 未確認・次の候補

- Dreamcast の 1 パケットの手間（模擬の 340 µs）は推定。x3 の落ち着いた速さで確かめる。
- (A) の窓 20 KiB での実効：Windows がゼロウィンドウでなく小さい窓（< MSS）でも送り止めるか、窓の更新が半分では遅すぎないか（`p*_max_frame_us`）。
- (A) の次の手（未実装）：KOS に遅延 ACK（2 セグメントに 1 回）を入れると Dreamcast の送信が半分になる。BBA の受信リングを 32 KiB（`RX_BUFFER_SHIFT 2`）にする手もあるが、ドライバの置き換えになる（同じ置き換えの方法は使えるが範囲が広い）。
- (B) を製品に入れるときの NGTP の変更（§4.2）は未設計。`bba-sd-write.md` の R7 は「測ってから」とあり、この文書はその計測の準備。
- 関連：R166、R167、R175、R178、`docs/design/menu-app.md` §8（D6）、`docs/design/bba-sd-write.md`（R7）、`docs/research/g2-perf-plan.md`、`tools/g2bench/README_ja.md`。
