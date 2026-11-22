# GDEMU 6.0 ピン探し：最初の実機実行のチェックリスト（段階 0）

> ## 重要：この手順の間、GDEMU を Dreamcast に付けない
>
> - 段階 0 の実行は **Dreamcast から外した基板**を、**電流制限つきの電源**で給電して行う（README_ja.md §0-1 の 3・4、§6「Dreamcast に付けたまま走らせる」）。
> - **例外は手順 B（純正ファームのバックアップ）だけ**：fwbackup は純正ファームの基板を Dreamcast から給電して行う（`tools/fwbackup/README_ja.md`）。ピン探しのファームを書いた後は、二度と Dreamcast に付けない。
> - **基板は替えがきかない**（R143）。純正ファームはバックアップが無ければ戻せない。**迷ったら進めない。どれか 1 項目でも欠けたら、次の項目へ進まない。**
>
> このチェックリストは、`README_ja.md` §0〜§6、`tools/fwbackup/README_ja.md`、`docs/research/LOG.md` R143・R145 の内容を順番に並べ直したもの。
> 実機で確かめた事実は一つも無い（LOG・README にある「未確認」はそのまま未確認）。数値・手順で README に無いものは「方針」と書いた。

## A. 用意するもの

- [ ] **段階 0 のファームの ELF**（`tools/pinfind/mkstage.ps1 -Stage 0` が作る `tools/pinfind/out/netgd-pinfind-stage0-<SHA256 の先頭 8 桁>.elf`）。
      `mkstage` の最後に出る **ビットストリームの SHA-256 と「STAGE 0」の表示**を控える（手順 F-1 でファームのログと照合する）。
- [ ] **段階 1 のファームは今回は使わない**（`...-stage1-....elf` と名前で区別できる。書くのは段階 0 の結果が全部想定どおりになってから、別の回に）。
- [ ] debugprobe（Raspberry Pi Pico、debugprobe 2.2.3 以上、CMSIS-DAP v2）と **probe-rs 0.32.0 ちょうど**（fwbackup の前提）。
- [ ] 電流制限つきの電源（100〜150 mA の制限から。**制限の無い電源・12 V は使わない**。R143）、電流計（記録できるもの）、テスター。
- [ ] ロジックアナライザ（3.3 V の閾値、入力は高インピーダンス。README §5）、外付けプルアップ用の 10 kΩ 程度の抵抗、クリップ付きのハーネス（隣のピンとの短絡を防ぐ）。
- [ ] SD カードは**差さない**（抜く）。

## B. 純正ファームのバックアップ（これが無ければ先へ進めない）

README §0-1 の 1。詳しくは `tools/fwbackup/README_ja.md`。**この手順だけは、純正ファームの基板を Dreamcast に付けて、Dreamcast から給電する**。

- [ ] B-1 配線は SWDIO / SWCLK / GND の 3 本だけ。**RESET・ERASE・3V3 は繋がない**（fwbackup の README）。SWD ヘッダの 7 番（GND）は SD スロットの外枠との導通で決める。
- [ ] B-2 Dreamcast は BIOS メニューで止める（ディスクを読まない画面）。`python fwbackup.py check` を実行する。
- [ ] B-3 結果が `UNLOCKED_2E` であること。
      **`LOCKED_OR_NO_ACCESS`・`NO_TARGET`・`UNLOCKED_OTHER` なら実行しない**（README §0-1 の 1。LOCKED の純正は戻せない。続けるかはユーザーの判断で、2 枚目の基板を勧める。ERASE には触らない）。
- [ ] B-4 `python fwbackup.py dump --out out` を実行する（約 1〜3 分）。
- [ ] B-5 **照合する**：`python fwbackup.py inspect out/gdemu60_flash_281A0761_<日時>.bin`、`.sha256` と実ファイルの SHA-256 が一致すること、
      `.json` の範囲・読み回数・チャンクごとの CRC32 に異常が無いこと（`dump` は 2 回読んで比較し、読み戻しの SHA-256 照合までしてから最終名にする。不一致なら何も残らない）。
- [ ] B-6 バックアップ（`.bin`・`.sha256`・`.json`）を**複数の場所**（基板とは別の媒体）に保管する。コミット・アップロード・共有はしない（`.gitignore` 済み。fwbackup の README）。
- [ ] B-7 Dreamcast の電源を切り、ジャンパー線を外し、**基板を Dreamcast から外す**。以降、基板は Dreamcast に付けない。

## C. 無通電での導通確認（テスター）

README §0-1 の 2。**基板に電源が入っていない状態**で行う。

- [ ] C-1 PS（FPGA のコンフィグ）の 5 本：
      PA23 = DATA0 = FPGA の G2、PA24 = DCLK = G1、PA25 = nCONFIG = H3、PB1 = CONF_DONE = F16、PB2 = nSTATUS = F3。
      **特に nCONFIG が PA25 であること**（そうでなければ、ファームは FPGA を止められない）。今のファームは v5.5 の割り当てを仮定している（`cyclone::pinmap`）。
- [ ] C-2 MSEL1（F15）が PS の側（Low）に固定されていること。
- [ ] C-3 LED のピン（PB18）。
- [ ] C-4 **どれか 1 つでも想定と違ったら、ここで止める**（ファームの割り当てを直してから、ビットストリームごと作り直す）。結果を記録する。

## D. 基板の外での給電の予行（純正ファームのまま）

README §0-1 の 3、LOG R143「基板外での給電」。**まだピン探しのファームは書かない。** 具体的な電圧・電流の値は README に無いので、ここで測って決める。

- [ ] D-1 無通電で、CN1 の電源ピン・U4・Q1・MCU の VDDIO・SWD の 1 番の導通と、各レールと GND の間の抵抗を記録する。
- [ ] D-2 （Dreamcast に付けた状態で）レールの電圧を基板上で測り、記録する。
- [ ] D-3 CN1 の電源ピンに、D-2 と同じ電圧を電流制限つきの電源で入れる（100〜150 mA の制限から。12 V に触れない）。
      **純正ファームのまま基板の外で起動する**ことを確かめ、電流と電圧の基準値を取る。
      SWD ヘッダの 1 番から給電する案は、レギュレータへの逆流のおそれがあり**勧めない**（R143）。
- [ ] D-4 基準値（起動時の電流など）が取れ、電流制限に当たらず起動したこと。
      **当たったら止まる**（原因が分かるまで進まない）。

## E. ピン探しのファームを書く（純正が消える）

- [ ] E-1 B-5 のバックアップが照合済みであること。もう一度確認する。
- [ ] E-2 `tools/pinfind/mkstage.ps1 -Stage 0` の出力で、**ELF の名前に `stage0` が入っている**こと、表示された**ビットストリームの SHA-256**を控えること。
- [ ] E-3 書き込みは SWD（`probe-rs`、チップ名 `ATSAM3U2E`）。**ERASE 工程を含むので §0 が済んでから**（README §3-4。「手順は P6 で確定させる」とあり、書き込みコマンドの正式な手順はまだ確定していない。形は `probe-rs download --chip ATSAM3U2E <ELF>`）。
      **ERASE ピンには触らない・`--allow-erase-all` を付けない**は fwbackup の方針。書き込みの具体的なコマンドの確認はユーザーが行う。
- [ ] E-4 書き込み直後の `probe-rs` のリセットは「1 回目の実行」として数えない（README §3-5 の 2）。

## F. 段階 0 を走らせる

README §0-2（段階 0）・§3-5。段階 0 は FPGA が**何も駆動しない**設計。確かめることは、ファームの安全装置（期限・GPBR・nCONFIG）、PS のピン、未使用ピンの扱い。

### 走らせる前に

- [ ] F-0 **GDEMU は Dreamcast に付いていない**。SD は抜いてある。電流制限つきの電源（100〜150 mA から）、電流計、probe-rs（RTT のログ）を用意する。
      デバッガで CPU を**止めない**（止めている間は nCONFIG を下げる処理も動かない。README §6）。
- [ ] F-1 起動直後の RTT に `PF STAGE 0 (no outputs driven) bitstream sha256=<64 桁> (552352 bytes raw)` と `PF pin-finder: stage0=true ...` が出ること。
      `PF STAGE 1-3 (DRIVES balls; NOT stage 0)` や `stage0=false` なら**すぐ電源を切る**（段階 1 以降のファームを書いている）。
      `sha256=` が、E-2 で控えた SHA-256 と一致すること（一致しなければ電源を切る）。
- [ ] F-2 ログを `rtt.log` に保存し、電流を記録する（README §3-5 の `probe-rs attach --chip ATSAM3U2E <ELF> | tee rtt.log`）。

### ロジアナを付けるタイミング（任意だが方針）

README §5 は CN1 側（DC_* の 34 本）をロジアナで読むのを「MCU 側の段階 2 が済んだあとの別の回」としている。段階 0 では FPGA は何も駆動しない設計なので、
**段階 0 でロジアナを使うのは「何も動かない（Low に引かれない・振られない）ことの確認」が目的**で、これは README に書かれた手順ではなく、このチェックリストの方針。

- [ ] F-3 ロジアナは**基板に電源を入れてから付ける**（電源の入っていない基板に、電源の入ったロジアナやプルアップをつながない。README §5 の 4）。
      ファームは起動後 10 秒、FPGA を構成前のまま待つ（`PF waiting 10 s`）。その間に付ける。
- [ ] F-4 読む線には外付けのプルアップ（10 kΩ を基板の 3.3 V へ。CN1 側にはプルアップが無い。README §5 の 3）。クリップは隣のピンと短絡させない。サンプリングは 1 MHz 以上。
- [ ] F-5 ロジアナを外すのは `PF end (stage 0)` の後、**電源を切る前**。

### 何を見るか（README §0-2、§4）

| 見るもの | 想定（段階 0 で正常） | 想定と違うとき |
|----------|----------------------|----------------|
| `PF waiting 10 s ...` の間の電流 | 予行（D-3）の基準から大きく増えない | 増えたら止める |
| `PF FPGA configured (... ms, ... bytes ...)` | 出る。`bytes` はビットストリームの大きさと一致（CONF_DONE が上がる） | 出ない・`PF stop: ...`・`PF FPGA load failed` なら止める（FPGA は nCONFIG Low になっている）。CONF_DONE が毎回ストリームの終わりより早く上がる場合は、正常かどうか未確認：判断が要る（README §4） |
| 構成後の電流 | 予行の基準から大きく増えない | 大きく増えたら**すぐ電源を切る** |
| `PF across configuration: 0 watched line(s) changed` | 0 本 | `PF diff across-configuration ...` が出たら、Supra が未使用ピンや hold のピンを出力にしている疑い。**先に進まない**（README §0-2） |
| 保持中（20 秒）の `PF diff during-stage0 ...` | 出ない | 出たら同上 |
| `PF end (stage 0): 0 change(s) while configured. FPGA held in reset (nCONFIG low). Power off.` | 0 change | 1 以上なら先に進まない |
| ロジアナ（F-3 以降を行った場合） | どの線も Low に引かれない・変化しない | 変化があれば、そのピンを記録して先に進まない |

### 停止条件（どれか 1 つでも当てはまったら、電源を切って止める）

- 電流が電流制限に当たった、または予行（D-3）の基準から大きく増えた。
- `PF stop: ...`、`PF FPGA load failed`、`CONF_DONE early or short transfer`、`PF diff ...` のいずれかが出た。
- ログが途中で止まった、`stage0=false` が出た、ビットストリームの SHA-256 が一致しない。
- 保持の 20 秒＋余裕（約 3 秒）を過ぎても `PF end (stage 0)` が出ない（ファームの独立の期限が働いて nCONFIG を Low にするはずだが、働いたことを確認できなければ電源を切る）。
- 基板に熱・におい・異音など、想定外のことがあった。
- ファームが `a pin-finder run already started since power-up (GPBR3)` で止まったときは、それは異常ではなく 2 回目の実行の防止。**電源を切る**（GPBR が消える）か、README §3-5 の 2 の方法で消す。

### 終わったら

- [ ] F-6 `PF end (stage 0)` を確認し、ロジアナ・プルアップを外してから電源を切る。
- [ ] F-7 `rtt.log`・電流の記録・ビットストリームの SHA-256・`output/pinfind_ag16k.txt`・ロジアナの記録を残す（README §0-2）。

## G. 段階 0 のあと

- 3 つ（`PF FPGA configured`・電流が基準から増えない・`PF across configuration: 0`／20 秒の保持中に `PF diff` なし）が**すべて想定どおり**になるまで、段階 1 に進まない（README §0-2）。
- 段階 1 は `mklist.py --max-drive 4` → `gen.py`（既定）→ `build.sh`、ファームは `mkstage.ps1 -Stage 1`。別の回に、結果を見てから決める。
- 結果と判断は、ユーザーが LOG に記録する（このチェックリストでは LOG を編集しない）。

## 付録：出典

| 項目 | 出典 |
|------|------|
| 必須の前提（バックアップ・導通・給電の予行・実行中の条件） | `README_ja.md` §0-1、LOG R143 |
| 段階 0 の確認事項 | `README_ja.md` §0-2、LOG R145 |
| ファームの流れ（10 秒待ち・GPBR3・期限） | `README_ja.md` §3-5、LOG R145 |
| ロジアナ・プルアップ | `README_ja.md` §5 |
| 電気的なリスク | `README_ja.md` §6、LOG R143 |
| バックアップの手順・`check` の結果 | `tools/fwbackup/README_ja.md` |
