# Supra 2026.03（Windows 版）の静的解析（2026-11-20、LOG R92）

ユーザーの許可（「ダウンロード許可します。解析して」）を受けて、AGM の FPGA 開発ツール Supra の Windows 版 zip を取得した。
スクラッチパッドの専用フォルダで、**中のファイルは一切実行せずに**調べた。担当は Opus。
ファイルは Supra の再配布にあたりうるので、リポジトリには入れていない。

## 1. 入手したもの

| 項目 | 値 |
|---|---|
| ファイル | `Supra-2026.03.b0-a052d0a8-win64-all.zip` |
| 入手元 | Internet Archive `https://archive.org/download/agm-micro/FPGA/2026.03/`（AGM 公式は百度網盤だけで、そちらからは取っていない） |
| 大きさ | 171,183,087 B |
| SHA-256 | `81819687762a7cfca1057ad6054703eedf6a83b950080633abe121fdeaf5e2c4` |
| MD5 / SHA-1 | `ab66cd3bac556cfef453fd3c36015808` / `9e7c1a90404190433e861e030659150bc1fe316e` |

Internet Archive のメタデータ（`archive.org/metadata/agm-micro`）にある MD5・SHA-1・サイズと一致した。
これで分かるのは「アーカイブにあるものと同じ物を受け取った」ことだけで、**AGM の公式の配布物と同じかどうかは確かめていない**。

## 2. 中身

- zip の項目は 1,921（ファイル 1,827）。危ないパス（絶対パスや `..`）は無く、zip の CRC もすべて正常。展開すると 267 MB。
- 構成：
  - `bin/`：`Supra.exe`（GUI、Qt）、`af.exe`（コマンドラインの配置配線）、`Downloader.exe`（書き込み）、`af_*.bat`
  - `map/bin/yosys.exe`：合成。Yosys 0.61 の派生版
  - `etc/`：`AG16KDDF256.pinmap` ほかのピン表、`af_map.tcl` などの手順スクリプト、`arch/`
  - `lib/tcl8.6`：Tcl の実行環境
  - そのほか：`openocd/`、`urjtag/`、`flashloader/`、`python_dist/`（Python 3.11.1）、`zadig-2.7/2.8.exe`、`license/license.txt`
- 文書は `doc/INSTALL.txt`・`MANUAL.txt`・`RELEASE.txt`・`LED_GUIDE.txt`（すべて英語）。

## 3. 署名

| 種類 | 結果 |
|---|---|
| **AGM 製**：`Supra.exe`・`af.exe`・`Downloader.exe`・`yosys.exe`・`agrv32flash.exe` | **署名なし** |
| 同梱のオープンソース：`openocd.exe`・`jtag.exe`・libusb・libftdi・capstone・hidapi・pthread | 署名なし |
| FTDI の `ftd2xx*.dll` | FTDI の正しい署名 |
| `python_dist`（Python 3.11.1、OpenSSL 1.1.1q、SQLite） | Python Software Foundation の正しい署名 |
| `vcruntime140*.dll` | Microsoft の正しい署名 |
| `zadig-2.7/2.8.exe` | Akeo Consulting の正しい署名 |
| `libusb0.dll` | 正しい署名 |
| `urjtag/inpout32.dll` | 署名なし（中身に Verisign の署名チェーンの文字列は入っている） |

AGM 製の実行ファイルに署名が無いので、改ざんされていないことを署名では確かめられない。

## 4. 使っている API（インポート表、`llvm-objdump -p`）

| ファイル | 気になる API | 判断 |
|---|---|---|
| `af.exe`・`Supra.exe`・`yosys.exe` | WS2_32（socket・connect・listen など）、NETAPI32（`NetGetDCName`・`NetUserGetInfo`）、`GetProfilesDirectoryW`、`AccessCheck` などのセキュリティ関数 | **同梱の Tcl の実行環境が使うもの**（Tcl の `socket` コマンド、`~user` のパス展開、`file` の属性）と同じ組み合わせ。`yosys.exe` にもまったく同じ組み合わせがある。単独では怪しくない |
| `af.exe`・`Supra.exe` | `GetAdaptersInfo`（IPHLPAPI） | ネットワークアダプタの MAC アドレスを読む。ノードロック型ライセンス（`license.txt` が `LICENSE_NODE_LOCKED`）の機械の識別とみられる。**PC の MAC アドレスはライセンス要求ファイルに入る可能性がある** |
| `Supra.exe` | `RegCreateKeyEx`・`RegSetValueEx`・`RegDeleteKey` | 設定の保存（Qt の `QSettings`）とみられる |
| `Supra.exe` | `SetWindowsHookExW` | Qt・Tk がダイアログのために使う形と同じ。キー入力の記録に使われる WH_KEYBOARD_LL かどうかは引数を見ないと分からない（未確認） |
| `Supra.exe` | `CheckRemoteDebuggerPresent` | デバッガの検出。ライセンス保護によくある |
| `af.exe`・`Supra.exe` | `CreateProcessW`・`ShellExecuteW`・`LoadLibrary`・`OpenProcess` | 外部ツール（yosys・Quartus・openocd）の呼び出しに使うとみられる |
| **`urjtag/inpout32.dll`** | `OpenSCManager`・`CreateService`・`StartService` | **カーネルドライバ（`inpoutx64.sys`、資源の中に入っている）を入れて動かす**。パラレルポート用の古い公開ドライバで、悪用の知られた種類（脆弱なドライバを入れる BYOVD の材料になる）。使うのは urjtag でパラレルポートの JTAG を使うときだけ |

プロセスへの注入に使う API（`WriteProcessMemory`・`CreateRemoteThread`・`VirtualAllocEx`）は、どのファイルにも無かった。

## 5. 文字列と構造

- **埋め込みの URL・IP アドレス**：
  - `af.exe`・`agrv32flash.exe`：無し
  - `Supra.exe`・`Downloader.exe`：Qt・XML・Adobe の名前空間の URL だけ。ドメインの一覧（`dyndns.org`・`*.ru` など）は Qt に入っている公開サフィックスの表で、通信先ではない
  - `yosys.exe`：論文と GitHub の URL だけ
  - `inpout32.dll`：署名の CRL と OCSP の URL だけ
  - AGM のドメイン（`alta-gate.com`）は、`Supra.exe` の中の組織名（QSettings 用）として1回出てくるだけ
- **パッカー**：どの実行ファイルも普通のセクション構成（`.text`・`.rdata`・`.pdata` など）で、エントロピーは 6.0〜6.4。UPX などで固めた跡は無い
- **スクリプト**：`bin/*.bat` はどれも PATH を通して `af.exe`・`yosys.exe` などを呼ぶ4〜5行だけ。PowerShell・certutil・bitsadmin・`reg add`・`schtasks` を使うスクリプトは無い（grep で当たったのは openocd の上流の設定ファイルだけ）
- **ライセンス**：`license/license.txt` は `LICENSE_NODE_LOCKED <40 桁の 16 進> "2036-03-31 07:25:13" <40 桁の 16 進>` の1行。`doc/INSTALL.txt` によると、試用版は期限付きのファイルを取り込む方式。正式版は要求ファイルを `license@alta-gate.com` にメールで送り、届いたライセンスを取り込む方式（Supra が自分で通信する手順は書かれていない）。同梱のファイルがこの PC で使えるかは、実行しないと分からない

## 6. ウイルススキャン

- Windows Defender はこの PC では止まっている（`AMRunningMode: Not running`）。有効なのは ESET Security。
- ESET のコマンドラインスキャナ `ecls.exe /clean-mode=none`（駆除しない、報告だけ）で zip と展開したフォルダを走査した。検出は **0**（ファイル 1,827、オブジェクト 72,736、17 秒）。定義は scanner 34194（20261120）。
- 展開のとき ESET のリアルタイム保護も止めなかった（ファイルが消されていない）。

## 7. NetGD にとって分かったこと（R69 の訂正）

- **Supra 2026.03 の Yosys（0.61）には `synth_agm` が無い**。入っているのは上流と同じ `synth_*` だけ（`synth_intel`・`synth_gowin` など）。AGM 向けの写像は `etc/af_map.tcl`（`yosys -import` で Tcl から Yosys のパスを並べる）と `etc/arch/alta-*.ar` で行う。
  R69 の「道 (1) `yosys synth_agm -vqm` → `af`」は古い版（rodinia が使った版）の話で、この版では `af_map.tcl` の流れになる。
- `etc/AG16KDDF256.pinmap`（214 行）があり、GDEMU 6.0 の FPGA の型番に対応している。
- 主な実行ファイルの SHA-256（今後の版と比べるため）：
  - `af.exe`：`66c5ec531adc11c75ae41864232cf157180923d1f6daccec2c67de62df56c15b`
  - `Supra.exe`：`78c4c26e3fb9c7331694caf2932910251fa80186555689295de1b17e65b0ec23`
  - `Downloader.exe`：`d3d2c8a12344bbb54b4d295fc42d70ed517b20cc43ea8c943ebd3a1b099e51f4`
  - `yosys.exe`：`7ff87c7259fb1dea02a3dba38ac303dea3f5cb07badcd5cffa33bfea2187301c`
  - `inpout32.dll`：`494cf02a528532e83165b6282a9268e78f8b20bb278d9e1ddf77df9091fa105d`

## 8. 結論と勧め

- **静的に見る限り、マルウェアらしい兆候は見つからなかった**：埋め込みの通信先が無い、パッカーが無い、注入の API が無い、ダウンローダーのスクリプトが無い、ESET の検出 0。
  ただし、**静的解析では「バックドアが無い」とは言い切れない**。AGM 製の主な実行ファイルは署名が無く、公式の配布物と同じかどうかも確かめていない。
- 注意点：
  1. `urjtag/inpout32.dll` はカーネルドライバを入れる。パラレルポートの JTAG は使わないので、**urjtag は使わない**。`zadig` も USB ドライバを差し替えるので、書き込み器を使うときだけにする。
  2. ノードロックのライセンスは MAC アドレスを使うとみられる。正式版のライセンスを要求すると、PC の識別情報が AGM に渡る。
  3. 動かすなら、**ネットワークを切った仮想マシンか別の Windows** で。NetGD に必要なのは合成と配置配線（`af.exe`・`yosys.exe`）だけで、書き込み器・ドライバ・GUI は要らない。
- 次の段階（実行するかはユーザーの判断）：隔離した環境で `af.exe --help` と、試験用の小さな設計を `af_map.tcl` に通す。通信が出ないかを見る。そのうえで NetGD の AG16K 版を通し、ビットストリームの実サイズを測る。
