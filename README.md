# NetGD

**Dreamcast の GD-ROM ドライブを置き換える基板（GDEMU 系）向けの、オープンソースのファームウェアと FPGA コア。**
[OpenGDEMU](https://github.com/cepawiel/OpenGDEMU)（Colton Pawielski 氏）のフォークです。

*Open-source firmware and FPGA core for GDEMU-style Dreamcast GD-ROM replacement boards, forked from
[OpenGDEMU](https://github.com/cepawiel/OpenGDEMU). Goals: run on GDEMU 6.0 boards, write game images to the
SD card over the Broadband Adapter, and match the original GDEMU's features with better performance.
Nothing is verified on hardware yet.*

> [!WARNING]
> **開発中です。このフォーク（NetGD）としては、まだ実機で動作を確認していません。**
> （コード中のコメントには、上流が実機で確認した事柄も書かれています。）
> 書き込むと純正ファームウェアは消え、元に戻す方法はありません（純正ファームは吸い出せません）。
> 壊れても構わない基板以外には書き込まないでください。

## 目標

| # | 目標 | 状況 |
|---|------|------|
| **G1** | **GDEMU 6.0 基板で動く**（上流は v5.5 / v5.15b クローンのみ） | 基板を調査済み：MCU は ATSAM3U2E、FPGA は **AGM AG16KDDF256**（v5.5 は Cyclone II）。FPGA の移植が本体 |
| **G2** | **ブロードバンドアダプタ（BBA）経由で SD カードにゲームイメージを書き込む**（本家 GDEMU では不可） | 設計済み。ファーム側の土台（SD 全体を IDE ディスクとして公開）はある |
| **G3** | **本家 GDEMU と同等の機能を持ち、より高性能** | 番号フォルダ方式と、GDmenu / openMenu の切替コマンド `0x52` を実装（実機未検証）。残りは `GDEMU.ini`、活線挿抜など |

詳細な計画（フェーズ、モデルの役割分担、未解決の課題）は [`docs/PLAN.md`](docs/PLAN.md) にあります。

## 構成

```
mcu/opengdemu/      MCU ファームウェア（Rust + embassy、SAM3U4E / SAM3U2E）
mcu/embassy-atsam3/ SAM3U 用 HAL
mcu/fwupdater/      RAM で動くファーム更新プログラム（ファームに埋め込み）
common/discimg/     GDI / CDI 解析と ATAPI 応答（no_std、ホストでテスト可能）
common/lzma/        FPGA ビットストリーム用の LZMA 展開
common/gamecfg/     GAME.CFG の解析
fpga/amaranth/      FPGA コア（Amaranth HDL）とシミュレーション
fpga/Quartus/       Cyclone II 用 Quartus プロジェクト（v5.5 クローン向け）
docs/               計画・調査・設計・基板の資料
```

パス名やバイナリ名（`mcu/opengdemu`、`opengdemu.elf`）は上流のままです。改名は後で行います。

## 今のファームウェアにできること（コード上。NetGD としては実機未確認）

- **起動**：LZMA 圧縮した FPGA ビットストリームを展開しながら FPGA に送り込み、SD カード（MBR + FAT32）を読んで既定のイメージをマウントする。
- **GD-ROM（プライマリ）**：GDI / CDI イメージから `CD_READ` を PIO・DMA で返す。TOC・セッション情報、CD 音声（CDDA）、`START STOP UNIT` による取り出し。
- **セカンダリ IDE ディスク**：SD カード全体を読み書き可能な ATA ディスクとして見せる（LBA28/48、PIO・DMA）。イメージのマウント中は書き込み禁止。
- **番号フォルダ**：本家と同じく SD のルートに `01`、`02`… のフォルダを置く。設定ファイル（`GDEMU.CFG`）が無ければ `01`（メニュー）をマウントする。
- **イメージ選択**：本家の切替コマンド `0x52`（GDmenu / openMenu が使う：番号指定、次／前、バージョン）と、独自コマンド `0xE0`（フォルダ名でマウント）。
- **ボタン**：`GAME.CFG` に `next` があればそれに従い、無ければ次の番号のフォルダへ（最後の次は `01`）。
- **ファーム更新**：セカンダリへの独自コマンド `0xF9` で、RAM 上の更新プログラムに切り替えて書き換える。

本家との残りの差（`GDEMU.ini`、活線挿抜、SD からのファーム更新など）は [`docs/research/gdemu6-and-parity.md`](docs/research/gdemu6-and-parity.md) にあります。

## ビルドとテスト

必要なもの：Rust（`rustup` が `rust-toolchain.toml` からツールチェーンと `thumbv7m-none-eabi` を入れる）、`xz`、Python 3（FPGA シミュ用）。

```sh
# ファーム（SAM3U4E、v5.5 クローン）
cd mcu/opengdemu && cargo build --release

# ファーム（SAM3U2E、GDEMU 6.0 の MCU）
DEFMT_LOG=off cargo build --release --features sam3u2e

# ホストテスト
(cd common/discimg && cargo test)
(cd common/lzma && cargo test)
(cd common/gamecfg && cargo test)

# FPGA シミュレーション
cd fpga/amaranth
python -m venv amaranth_env && amaranth_env/bin/pip install "amaranth==0.5.8" "amaranth-yosys==0.50.0.0.post123"
amaranth_env/bin/python test.py   # test_cdda.py / test_timing.py / test_top.py も
```

- `cargo build` で `--target-dir` を変えないでください（リンカの `-Map=target/openmenu.map` が相対パスのため失敗します）。
- Cyclone II のビットストリームは Quartus II 13.0.1 Web Edition をコンテナで動かして作ります（`Containers/QuartusII_x64`、`fpga/build_fpga.sh`）。
- CI（`.github/workflows/build.yaml`）が、ファームのビルド、ホストテスト、FPGA シミュ、生成済み Verilog の一致をチェックします。

書き込み手順（SWD プローブの準備、純正ファームの消去、書き込み）は [`INSTALL.md`](INSTALL.md) を参照してください（v5.5 クローン向け）。

## 資料

| 資料 | 内容 |
|------|------|
| [`docs/PLAN.md`](docs/PLAN.md) | 計画：目標、フェーズ、役割分担、既知の地雷 |
| [`docs/research/LOG.md`](docs/research/LOG.md) | **すべての調査・レビュー・判断の記録** |
| [`docs/research/`](docs/research/) | GDEMU 6.0 と本家機能の調査、web 調査（openMenu の `0x52`、AG16K、KOS g1ata など） |
| [`docs/design/bba-sd-write.md`](docs/design/bba-sd-write.md) | BBA 経由の SD 書き込みの設計 |
| [`docs/boards/gdemu60/`](docs/boards/gdemu60/) | GDEMU 6.0 基板の写真と部品 |
| [`docs/PCB.md`](docs/PCB.md) | v5.5 / v5.15b クローン基板の解析（上流） |
| [`docs/AddressMapMCU.md`](docs/AddressMapMCU.md) | MCU から見た FPGA レジスタ |

関連：BBA での実機開発ツール [Dreamcast-Dev-Tools-for-BBA](https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA)（G2 の PC 側で使う予定）。

## クレジットとライセンス

- [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU)（Colton Pawielski 氏）— NetGD のフォーク元。上流の目標（イメージの取り出し、セカンダリ IDE ディスク、パス指定の再マウント）は実装に引き継いでいます。
- [Amaranth](https://github.com/amaranth-lang/amaranth)（BSD-2-Clause）
- [embassy](https://github.com/embassy-rs/embassy)

ライセンスは上流と同じ **GPL-3.0**（[`LICENSE`](LICENSE)）です。帰属表記に漏れがあれば Issue でお知らせください。
