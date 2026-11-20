# AG16K のビットストリームを作る道筋（2026-11-20 時点、実機なし）

担当：Sonnet（調査と環境確認のみ。コード・LOG・PLAN は変更していない）
対象：`fpga/agm/output/OpenGDEMU_ag16k.rbf` を作るために何が要るか。前提は `docs/design/g1-ag16k-port.md`、`docs/research/ag16k-config.md`、LOG R39。
信頼度：**[測定]** このPCで実行 / **[一次]** 文書・ソース・ページを直接読んだ / **[二次]** フォーラム・検索要約 / **[推測]** / **[不明]**。

---

## 0. 結論

1. **このPCでビットストリームまでは作れない。** 理由は 2 つ。(a) 合成できる Yosys が無い（下の §1）。(b) AG16K 向けの配置配線ができるのは AGM の Supra（`af`）だけで、未入手。
2. **オープンな配置配線は無い。** nextpnr は AG1KLPQ48 だけが作業中で、AG16K は無い [一次]。ビットストリームの解読（`ag16k-wires.json.gz`）はあるが、AG16KSDE176・非圧縮のみで、DDF256 は対象外。
3. **道は 2 本、どちらも Supra が要る。** Supra は AGM 配布のトライアルウェア。Windows 版（`af.exe`、コマンドライン可）と Linux 版がある [一次/二次]。DL 元は §4 の URL。**持ち主の操作が要る（私は実行ファイルを取得していない）。**
4. **サイズ**：非圧縮 `.rbf` = `.bin` = **552,366 B** [一次：AG16KSDE176 の実ビルド。DDF256 で同じかは未確認]。LZMA 後は **55〜75 KB の見積もりのまま**（[推測]。実ビルドが無いので更新できない）。2E のビットストリーム上限は 67,020 B（`g1-ag16k-port.md` §3-1）なので、入るか否かはまだ決まらない。
5. R39 の「5/5 メモリ、14 M9K、LUT 2,821」は YoWASP 0.71 での数字で [測定]、今回は**再現できなかった**（ツールが無い）。`synth_agm` での確認は Supra を入れてからになる。

## 1. このPCにあるもの [測定]

| もの | 結果 |
|------|------|
| `fpga/amaranth/amaranth_env` の `amaranth_yosys` 0.50（wasm） | `python -m amaranth_yosys -V` は動く。**最小ビルドで、`read_verilog`・`synth*`・`stat` が無い**（`help` に出るのは `read_rtlil`、`proc*`、`opt`、`memory_collect`、`write_json/verilog/rtlil/cxxrtl` だけ）。`synth_intel` も `synth_agm` も不可。Amaranth の Verilog 出力専用 |
| システムの `yosys`・`nextpnr-*`・`quartus_sh`（Windows、Git Bash） | 無し |
| WSL（Ubuntu 24.04、Python 3.12、32 コア・31 GB） | `yosys`・`nextpnr`・`verilator`・`docker`・`podman` は無し。`sudo` はパスワード要（`apt install` 不可） |
| `Containers/QuartusII_x64` | Containerfile だけ。**docker も podman も無いのでビルド不可**。さらに 4.5 GB の Quartus 13.0.1 Web を `download.altera.com` から取る作り |
| `fpga/Quartus/output_files/OpenGDEMU.rbf` | 73,821 B（v5.5 用、Cyclone II） |
| `fpga/agm/` | 存在しない（`g1-ag16k-port.md` §2-1 の設計のみ） |

**試みて止めたこと**：R39 と同じ YoWASP Yosys（PyPI の `yowasp-yosys`）を WSL の venv に入れようとしたが、環境の承認機構に拒否された。**持ち主が許可するなら** `python3 -m venv ~/ag16k/v && ~/ag16k/v/bin/pip install yowasp-yosys`（PyPI の公式ホイール、約 数十 MB）で入る。これで `synth_intel -family cycloneive` の再現（R39）はできるが、**AGM 固有の `synth_agm` や `alta_bram9k` への推論は確認できない**（それは Supra 付属の Yosys だけ）。

Amaranth 側のコードは今回一切変更していない（合成で何も走らせていないので、バグの証拠も無い）。`git status` で `fpga/` は無変更。

## 2. 調べたこと：どんな道があるか

### 2-1 Supra とは

- AGM Micro のプロプライエタリな FPGA ツール（GUI `Supra.exe` + コマンドラインの `af`）。Windows 版と Linux 版（`-all.tar.gz`）がある [一次：アーカイブの一覧]。
- ライセンスは **トライアルウェア** [二次：scoop の manifest（`agm-supra.json`、版 2020.07、`"license": "Trialware"`）]。EEVblog では「zip にライセンスファイルが入っている」との書き込みがあるが、**ドングルや有償条件の文面は見つからなかった**。再配布は不可と見るべきで、CI には載せられない（`g1-ag16k-port.md` §2-6 のとおり）。
- 最新の配布物（2026.03）の Windows 版の中身（アーカイブの一覧を読んだだけ）：`bin/af.exe`（13.5 MB）、`bin/Supra.exe`、`bin/Downloader.exe`、`etc/AG16KDDF256.pinmap`（3,734 B）、`AG16KF256.pinmap`、`AG16KSDF256.pinmap` ほか、`etc/af_quartus.tcl`・`af_quartus.qsf`・`af_run.tcl`、`etc/arch/rodinia/route/route.AG10K.bar.gz`（28 MB）・`route.AG15K.bar.gz`（69 MB）、`flashloader/`、`doc/MANUAL.txt`・`INSTALL.txt`・`RELEASE.txt`。**`AG16KDDF256.pinmap` があるので、Supra は DDF256 というデバイス名を知っている**（`g1-ag16k-port.md` U6 の一部が解けた）。ただし一覧は途中で切れており、**`map/`（付属 Yosys）がこの zip にあるかは未確認**。2019.10 の Linux 版にあった `map/bin/yosys` は Windows 版の一覧に見えなかった。

### 2-2 流れ 1：Supra 付属 Yosys（`synth_agm`）→ `af`（rodinia の流れ）

出典：`pablomarx/rodinia`（https://github.com/pablomarx/rodinia）の `examples/blinky10k/synth.sh` ほか [一次]。

```
yosys -p 'synth_agm -top OpenGDEMU -vqm OpenGDEMU.vqm' OpenGDEMUCore_ag16k.v OpenGDEMU_agm.v alta-bb.v
af -B --batch af_run.tcl   # load_architect → read_design_and_pack → read_sdc → place_and_route_design → bitgen normal -prg … -bin …
```

- `synth_agm` は **オープンな Yosys には無い**（このPCの 0.50 にも、R39 の 0.71 にも無い）。Supra 付属の Yosys のフォークにだけある [一次]。
- rodinia で動かしている実例は AG1K / AG10K。**AG16K をこの流れで通した記録は見つからなかった**（AG16K は nextpnr の実験だけ）。
- `.bin` が基本の出力で、`.rbf` は `.bin` の各バイトのビット順を反転したもの [一次：`ag16k-config.md` §2]。

### 2-3 流れ 2：Quartus で合成 → Supra に取り込む（AGM が公式に示す DDF256 の手順）

出典：`MANUAL_AG16KDDF256.pdf`（https://archive.org/download/agm-micro/MANUAL_AG16KDDF256.pdf）、`ag16k-config.md` §9 [一次]。

- 手順：Quartus で **EP4CE15F23**（Cyclone IV E）を対象に合成 → ピンは `.ve` ファイルで別に割り当て → Supra の Prepare / Migrate → 配置配線 → ビットストリーム。Supra にはほかに「Native」（付属 Yosys で合成）もある。
- Quartus の版：EEVblog の報告では **13.1 と Supra 2023.02 で動いた**、**24.1 Standard で動いた**、**Web / Lite 版は設計パーティションが作れず AGM の PLL・DAC が最適化で消える**（`modeQuartus=true` が要る、Native は配置配線でエラー）[二次：https://www.eevblog.com/forum/fpga/agm-cplds-fpgas/ の #8・#10・#15・#17・#18・#24]。
- **この道に Quartus 13.0.1 Web（`Containers/QuartusII_x64`）が使えるかは不明**：Web 版で PLL が消える報告に当たる恐れがある。Cyclone IV E 自体は 13.0sp1 が対応するはず [推測]。
- 利点：メモリの推論を Quartus に任せられる（FIFO の `alta_bram9k` 化問題を回避できる）。欠点：Quartus（有償 Standard か 4.5 GB の Web）と Supra の両方が要る。

### 2-4 オープンな道

| 候補 | 状況 |
|------|------|
| nextpnr（himbaechel / generic） | rodinia の `nextpnr/` は **AG1KLPQ48 だけ**、作業中 [一次]。AG16K 用のチップデータベースは無い |
| Yosys の `synth_intel` | Cyclone IV の規則で合成はできるが（R39）、出るのは Quartus 用ネットリスト。AG16K に配置配線する先が無い。**資源の見積もり用（LUT/FF/メモリ数）だけ** |
| ビットストリーム解読 | `bitstream/agm-unpack.py`・`ag16k-wires.json.gz`（rodinia）。AG16KSDE176 の非圧縮のみ。**ビットストリームを自分で組み立てる道は現実的でない** |
| AgRV_pio（PlatformIO パッケージ）内の Supra | 第三者の再配布。EEVblog で「PlatformIO は動かなかった」という報告あり [二次] |

→ **ビットストリームを出す道は Supra を使う 2 本だけ。**

## 3. 手順（持ち主が Supra を入れた後）

1. **入手**（持ち主）：§4 の URL から Windows 版 zip を取り、SHA-256 を記録して `E:\tools\supra\`（例）に展開。実行ファイルはリポジトリに入れない。ライセンスの文面を読み、`docs/research/` に要旨を残す。
2. **疎通**：rodinia の `examples/blinky10k`（AG10K）と、AG16K（`AG16KDDF256` または `AG16KF256`）向けの LED 点滅を `af` で通す。`af -B --batch` が動き、`.bin` が 552,366 B 前後になることを見る。
3. **入力の用意**（Claude）：`fpga/agm/` を作り、`OpenGDEMU.v`（上位）から `core_pll` を外す（出力が未使用 [`g1-ag16k-port.md` §2-2]）、`Audio_PLL` を `af` の `generate_pll`（`alta_pllx`/`pllv`）で作り直す、仮のピン配置（`.asf`）と `.sdc` を書く。`OpenGDEMUCore_ag16k.v` は `fpga/amaranth/main.py --target ag16k` が出す。
4. **流れ 2-2**：`synth_agm` → `.vqm` → `af`。ここで **R39 の残る確認**を取る：`alta_bram9k` が 14 個前後か、FF が 2,000 未満か。外れたら流れ 2-3（Quartus 合成）か、`main.py` で AG16K 用のメモリ実装を足す（`g1-ag16k-port.md` §2-3 の対策 2）。
5. **出力**：`bitgen normal -bin` で `.bin`。`.rbf`（ビット順反転）を `fpga/agm/output/OpenGDEMU_ag16k.rbf` に置き、`BUILD.txt` に Supra の版・入力 Verilog の SHA-256・出力の SHA-256 を残す。
6. **サイズ測定**（基板不要）：非圧縮と、Supra の圧縮（LZW）ありの両方を作り、`xz --format=raw --lzma1=preset=9e,dict=4KiB,lc=0,lp=0,pb=0` にかけて 3 つの数字（非圧縮→LZMA、LZW、LZW→LZMA）を `g1-ag16k-port.md` §3-2 に追記。

詰まりやすい所：Native（付属 Yosys）で配置配線エラー、PLL・DAC が最適化で消える（EEVblog #10・#15・#17）。

## 4. 何が止まっているか

| 項目 | 状態 | 誰が |
|------|------|------|
| 合成用の Yosys（`synth_intel` 再現） | PyPI の `yowasp-yosys` を WSL に入れる操作が環境の承認で拒否された。許可があれば入る | 持ち主の許可 |
| **Supra の入手** | 下の URL。**実行ファイルなので私は取得していない**。AGM 公式は百度網盤のみ | **持ち主** |
| Supra のライセンス | トライアルウェアとの二次情報のみ。文面は未確認 | 持ち主（zip を開いて確認） |
| Quartus | Containerfile はあるが docker/podman が無い。4.5 GB のインストーラが要る | 持ち主 |
| `synth_agm` が M9K を推論するか | Supra を入れるまで不明 | Supra 後 |
| ピン配置（`.asf`） | `AG16KDDF256.pinmap` が Supra に入っている。MCU・G1 側の対応は基板（作業 2）待ち。**仮配置でもサイズ測定には十分** | 実機 |

**Supra の入手先（取得はしていない。持ち主が確認して入れる）**：

- AGM 公式：https://www.agm-micro.com/solutions.aspx?id=77&p=61 （百度網盤 `https://pan.baidu.com/s/1rJiAJHQQCp8WPGSPwCLm1Q?pwd=1234`）
- Internet Archive の写し（Windows）：https://archive.org/download/agm-micro/FPGA/2026.03/Supra-2026.03.b0-a052d0a8-win64-all.zip （163.3 MB）。Linux：`…/Supra-2026.03.b0-a052d0a8-all.tar.gz`（178.7 MB）。その前の版：`…/FPGA/2025.05/`。一覧：https://archive.org/download/agm-micro/FPGA/
- scoop の manifest（2020.07、`Trialware`）：https://github.com/SayCV/scoop-cvp/blob/main/bucket/agm-supra.json
- 注意：EEVblog には「中国製のバックドア入りアプリ」を避けたという書き込みがある（出所不明）。**ウイルススキャンと、隔離した環境（VM / 別の Windows）での実行を勧める**。

## 5. サイズの見積もり

| 対象 | 大きさ | 根拠 |
|------|-------:|------|
| `.bin`（非圧縮、PS に流すもの） | **552,366 B** | [一次] rodinia `blinky16k.bin`（AG16K、設計に依らず一定） |
| `.rbf`（ビット順反転） | 552,366 B | 同じ大きさ [一次] |
| ほぼ空の設計を LZMA（辞書 4 KiB） | 10,403 B | [測定] `g1-ag16k-port.md` §3-2 の下限 |
| **NetGD を LZMA** | **55〜75 KB** | [推測]。Cyclone II で非圧縮 158 KB → 44.6 KB だった比率と、デバイスが 3.5 倍広い分の位置情報の増加から。**今回も実ビルドが無いので確度は変わらない** |
| AGM の圧縮（LZW）あり | 不明 | AG16K は PS でも圧縮を受け付ける [一次：`AG10K_16K_guide.pdf`] |
| 2E の上限 | 67,020 B | `g1-ag16k-port.md` §3-1（コード 64,052 B を引いた残り） |

→ 見積もりの範囲が上限をまたぐ。**Supra で 1 回ビルドするまで決まらない**。入らない場合の順序は `g1-ag16k-port.md` §3-5 と `g1-bitstream-storage.md`。

## 6. 次の一手（持ち主の判断）

1. `yowasp-yosys` を WSL の venv に入れてよいか（R39 の再現と、Amaranth 側の合成上の問題の洗い出しができる。AGM 固有の確認はできない）。
2. Supra を入れる（上記 URL、隔離環境で）。入れた後は §3 の 2〜6 を Claude が実行できる（`af` は Windows のコマンドラインから動く見込み）。
