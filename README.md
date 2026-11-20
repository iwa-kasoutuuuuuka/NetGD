# NetGD

**Dreamcast の GD-ROM ドライブを置き換える基板（GDEMU 系）向けの、オープンソースのファームウェアと FPGA コア。**
[OpenGDEMU](https://github.com/cepawiel/OpenGDEMU)（Colton Pawielski 氏）のフォークです。

**Open-source firmware and FPGA core for GDEMU-style boards that replace the Dreamcast's GD-ROM drive.**
A fork of [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU) by Colton Pawielski.

> [!WARNING]
> **開発中です。このフォーク（NetGD）としては、まだ実機で動作を確認していません。**
> （コード中のコメントには、上流が実機で確認した事柄も書かれています。）
> 書き込むと純正ファームウェアは消え、元に戻す方法はありません（純正ファームは吸い出せません）。
> 壊れても構わない基板以外には書き込まないでください。
>
> **Work in progress. NetGD itself has not been run on real hardware yet.**
> (Some code comments describe things upstream verified on hardware.)
> Flashing erases the stock firmware for good — it cannot be dumped, so there is no way back.
> Only flash a board you can afford to lose.

## 目標 / Goals

| # | 目標 | Goal | 状況 / Status |
|---|------|------|---------------|
| **G1** | **GDEMU 6.0 基板で動く**（上流は v5.5 / v5.15b クローンのみ） | **Run on GDEMU 6.0 boards** (upstream targets v5.5 / v5.15b clones only) | 基板を調査済み：MCU は ATSAM3U2E、FPGA は **AGM AG16KDDF256**（v5.5 は Cyclone II）。FPGA の移植が本体 / Board surveyed: ATSAM3U2E MCU and an **AGM AG16KDDF256** FPGA instead of Cyclone II; the FPGA port is the main work |
| **G2** | **ブロードバンドアダプタ（BBA）経由で SD カードにゲームイメージを書き込む**（本家 GDEMU では不可） | **Write game images to the SD card over the Broadband Adapter** (the original GDEMU cannot) | 設計済み。ファーム側の修正（F1〜F8）は実装済み、PC 側ツール netgd-send は実装済み、Dreamcast 側のコンソールアプリは作業中（いずれも実機未検証） / Designed; firmware fixes F1–F8 are in, the PC tool netgd-send is done, the console app is in progress (none hardware-tested yet) |
| **G3** | **本家 GDEMU と同等の機能を持ち、より高性能** | **Match the original GDEMU's features, with better performance** | 番号フォルダ方式、GDmenu / openMenu の切替コマンド `0x52`、`GDEMU.INI` を実装（実機未検証） / Numbered folders, the GDmenu / openMenu image command `0x52`, and `GDEMU.INI` are in (not yet hardware-tested) |

詳細な計画（フェーズ、モデルの役割分担、未解決の課題）は [`docs/PLAN.md`](docs/PLAN.md) にあります。

The full plan (phases, roles, open issues) is in [`docs/PLAN.md`](docs/PLAN.md) (Japanese).

## 構成 / Layout

```
mcu/opengdemu/      MCU ファームウェア（Rust + embassy、SAM3U4E / SAM3U2E）  MCU firmware (Rust + embassy)
mcu/embassy-atsam3/ SAM3U 用 HAL                                             HAL for the SAM3U
mcu/fwupdater/      RAM で動くファーム更新プログラム（ファームに埋め込み）  RAM-resident updater, embedded in the firmware
common/discimg/     GDI / CDI 解析と ATAPI 応答（no_std、ホストでテスト可能） GDI/CDI parsing and ATAPI replies (host-tested)
common/lzma/        FPGA ビットストリーム用の LZMA 展開                       LZMA decoder for the FPGA bitstream
common/gamecfg/     GAME.CFG などの設定ファイルの解析                         Config file parsing
fpga/amaranth/      FPGA コア（Amaranth HDL）とシミュレーション。`main.py --target ag16k` で AG16K（6.0）用の Verilog を出す   FPGA core (Amaranth HDL) and simulations; `main.py --target ag16k` emits Verilog for the AG16K (6.0)
fpga/Quartus/       Cyclone II 用 Quartus プロジェクト（v5.5 クローン向け）   Quartus project for Cyclone II (v5.5 clones)
tools/gdprobe/      純正 GDEMU を調べる読み取り専用プローブ（KallistiOS）     Read-only GD-ROM probe for the stock GDEMU (KallistiOS)
tools/mksdcard/       調査用 SD カードの中身一式を生成する（Python 標準ライブラリ）     Generates the whole test SD card for the probe (Python stdlib)
tools/fwbackup/       純正 GDEMU 6.0 の MCU フラッシュを読み取り専用で確認・バックアップ（SWD）。吸い出したファイルは非公開     Read-only check and backup of the stock GDEMU 6.0 MCU flash over SWD; dumps stay private
tools/netgd-send/     BBA 経由の SD 書き込みの PC 側（Python）     PC side of writes to the SD card over the BBA (Python)
tools/ci/             KOS ビルドのスクリプト（GitHub Actions から呼ぶ）     KallistiOS build script (called by GitHub Actions)
docs/               計画・調査・設計・基板の資料                               Plan, research, designs, board notes
```

パス名やバイナリ名（`mcu/opengdemu`、`opengdemu.elf`）は上流のままです。改名は後で行います。

Paths and binary names (`mcu/opengdemu`, `opengdemu.elf`) still carry the upstream name; renaming comes later.

## 今のファームウェアにできること / What the firmware does

コード上の内容です。NetGD としては実機で未確認です。

Per the code; not yet verified on hardware with NetGD.

| 機能 | 内容 | Feature |
|------|------|---------|
| 起動 | LZMA 圧縮した FPGA ビットストリームを展開しながら FPGA に送り込み、SD カード（MBR + FAT32）を読んで既定のイメージをマウントする | **Boot:** unpacks the LZMA-compressed bitstream straight into the FPGA, reads the SD card (MBR + FAT32) and mounts the default image |
| GD-ROM（プライマリ） | GDI / CDI イメージから `CD_READ` を PIO・DMA で返す。TOC・セッション情報、CD 音声（CDDA）、`START STOP UNIT` による取り出し | **GD-ROM (primary):** serves `CD_READ` from GDI/CDI images over PIO and DMA; TOC and session info, CD audio, eject via `START STOP UNIT` |
| セカンダリ IDE ディスク | SD カード全体を読み書き可能な ATA ディスクとして見せる（LBA28/48、PIO・MWDMA2）。イメージのマウント中は書き込み禁止 | **Secondary IDE disk:** exposes the whole card as a read/write ATA disk (LBA28/48, PIO and MWDMA2); writes are refused while an image is mounted |
| 番号フォルダ | 本家と同じく SD のルートに `01`、`02`… のフォルダを置く。設定ファイル（`GDEMU.CFG`）が無ければ `01`（メニュー）をマウントする | **Numbered folders:** `01`, `02`, … in the card root like the original; without `GDEMU.CFG`, folder `01` (the menu) is mounted |
| イメージ選択 | 本家の切替コマンド `0x52`（GDmenu / openMenu が使う：番号指定、次／前、バージョン）と、独自コマンド `0xE0`（フォルダ名でマウント） | **Image selection:** the original's `0x52` command used by GDmenu / openMenu (by number, next/previous, version) and NetGD's own `0xE0` (mount by folder name) |
| ボタン | `GAME.CFG` に `next` があればそれに従い、無ければ次の番号のフォルダへ（最後の次は `01`、イメージの無いフォルダは飛ばす） | **Button:** follows `GAME.CFG`'s `next`, otherwise steps to the next numbered folder (wrapping to `01`, skipping folders without an image) |
| 設定ファイル | `GDEMU.INI`（本家と同じ。`reset_goto`・`open_time`・`detect_time` に対応）と、NetGD 独自の `GDEMU.CFG`（`image=` で起動時のイメージ） | **Config files:** `GDEMU.INI` like the original (`reset_goto`, `open_time`, `detect_time`) plus NetGD's own `GDEMU.CFG` (`image=` for the power-on image) |
| BBA 書き込み用 | セカンダリへの書き込みセッション（`0xE0` の 6/7/8）、MBR の保護、書き込みデータの FNV-1a、FLUSH CACHE、MWDMA2 | **For BBA writes:** a write session on IMAGE_CTL (`0xE0` 6/7/8), MBR protection, per-command FNV-1a of written data, FLUSH CACHE, MWDMA2 |
| ファーム更新 | セカンダリへの独自コマンド `0xF9` で、RAM 上の更新プログラムに切り替えて書き換える | **Firmware update:** vendor command `0xF9` on the secondary hands over to the RAM-resident updater |

本家との残りの差（活線挿抜、SD からのファーム更新など）は [`docs/research/gdemu6-and-parity.md`](docs/research/gdemu6-and-parity.md) にあります。

Remaining gaps against the original (card hot-swap, firmware update from SD, …) are listed in [`docs/research/gdemu6-and-parity.md`](docs/research/gdemu6-and-parity.md).

## ビルドとテスト / Build and test

必要なもの：Rust（`rustup` が `rust-toolchain.toml` からツールチェーンと `thumbv7m-none-eabi` を入れる）、`xz`、Python 3（FPGA シミュ用）。

Requirements: Rust (`rustup` installs the toolchain and `thumbv7m-none-eabi` from `rust-toolchain.toml`), `xz`, and Python 3 for the FPGA simulations.

```sh
# ファーム（SAM3U4E、v5.5 クローン） / Firmware for SAM3U4E (v5.5 clones)
cd mcu/opengdemu && cargo build --release

# ファーム（SAM3U2E、GDEMU 6.0 の MCU） / Firmware for SAM3U2E (the GDEMU 6.0 MCU)
DEFMT_LOG=off cargo build --release --features sam3u2e

# ホストテスト / Host tests
(cd common/discimg && cargo test)
(cd common/lzma && cargo test)
(cd common/gamecfg && cargo test)
(cd tools/gdprobe && sh tests/run_tests.sh)
(cd tools/mksdcard && python tests/test_mksdcard.py)
(cd tools/netgd-send && python -m unittest discover -s tests -v)
(cd tools/fwbackup && FWBACKUP_TEST_WRAPPER=1 python -m unittest discover -s tests -v)

# FPGA シミュレーション / FPGA simulations
cd fpga/amaranth
python -m venv amaranth_env && amaranth_env/bin/pip install "amaranth==0.5.8" "amaranth-yosys==0.50.0.0.post123"
amaranth_env/bin/python test.py   # test_cdda.py / test_timing.py / test_top.py も / too
```

- `cargo build` で `--target-dir` を変えないでください（リンカの `-Map=target/openmenu.map` が相対パスのため失敗します）。
  Don't pass `--target-dir` to `cargo build`: the linker's `-Map=target/openmenu.map` is a relative path and the link fails.
- Cyclone II のビットストリームは Quartus II 13.0.1 Web Edition をコンテナで動かして作ります（`Containers/QuartusII_x64`、`fpga/build_fpga.sh`）。
  The Cyclone II bitstream is built with Quartus II 13.0.1 Web Edition in a container (`Containers/QuartusII_x64`, `fpga/build_fpga.sh`).
- KallistiOS（Dreamcast）向けのプログラム（`tools/gdprobe` など）は GitHub Actions（`.github/workflows/kos.yaml`）でビルドされます。ビルドされた ELF は `dreamcast-elfs` という artifact からダウンロードできます（ローカルに DreamSDK は要りません）。
  Programs for KallistiOS (Dreamcast), such as `tools/gdprobe`, are built by GitHub Actions (`.github/workflows/kos.yaml`). The built ELFs can be downloaded from the `dreamcast-elfs` artifact (no local DreamSDK needed).
- CI（`.github/workflows/build.yaml`）が、ファームのビルド、ホストテスト、FPGA シミュ、生成済み Verilog の一致をチェックします。
  CI (`.github/workflows/build.yaml`) builds the firmware and runs the host tests, the FPGA simulations, and a check that the committed Verilog matches its source.

書き込み手順（SWD プローブの準備、純正ファームの消去、書き込み）は [`INSTALL.md`](INSTALL.md) を参照してください（v5.5 クローン向け）。

Flashing (SWD probe setup, erasing the stock firmware, writing NetGD) is in [`INSTALL.md`](INSTALL.md) (English, for v5.5 clones).

## 資料 / Documents

ほとんどの資料は日本語です。 / Most documents are in Japanese.

| 資料 / Document | 内容 | Contents |
|------|------|----------|
| [`docs/PLAN.md`](docs/PLAN.md) | 計画：目標、フェーズ、役割分担、既知の地雷 | Plan: goals, phases, roles, known pitfalls |
| [`docs/research/LOG.md`](docs/research/LOG.md) | **すべての調査・レビュー・判断の記録** | **Log of every investigation, review and decision** |
| [`docs/research/`](docs/research/) | GDEMU 6.0 と本家機能の調査、web 調査（openMenu の `0x52`、AG16K、KOS g1ata など）、純正 GDEMU の調査フロー | Research on GDEMU 6.0 and the original's features, web findings, the stock-GDEMU probing workflow |
| [`docs/design/bba-sd-write.md`](docs/design/bba-sd-write.md) | BBA 経由の SD 書き込みの設計 | Design for writing to the SD card over the BBA |
| [`docs/boards/gdemu60/`](docs/boards/gdemu60/) | GDEMU 6.0 基板の写真と部品 | Photos and parts of a GDEMU 6.0 board |
| [`docs/PCB.md`](docs/PCB.md) | v5.5 / v5.15b クローン基板の解析（上流、英語） | Reverse engineering of v5.5 / v5.15b clone boards (upstream, English) |
| [`docs/AddressMapMCU.md`](docs/AddressMapMCU.md) | MCU から見た FPGA レジスタ（英語） | FPGA registers as seen by the MCU (English) |

関連：BBA での実機開発ツール [Dreamcast-Dev-Tools-for-BBA](https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA)（G2 の PC 側とプローブの実行に使う）。

Related: [Dreamcast-Dev-Tools-for-BBA](https://github.com/iwa-kasoutuuuuuka/Dreamcast-Dev-Tools-for-BBA), tooling for developing on real hardware over the BBA (used for G2's PC side and for running the probe).

## クレジットとライセンス / Credits and license

- [OpenGDEMU](https://github.com/cepawiel/OpenGDEMU)（Colton Pawielski 氏）— NetGD のフォーク元。上流の目標（イメージの取り出し、セカンダリ IDE ディスク、パス指定の再マウント）は実装に引き継いでいます。
  The project NetGD is forked from; its goals (ejecting images, a secondary IDE disk, remounting by path) carry over into the code.
- [Amaranth](https://github.com/amaranth-lang/amaranth)（BSD-2-Clause）
- [embassy](https://github.com/embassy-rs/embassy)

ライセンスは上流と同じ **GPL-3.0**（[`LICENSE`](LICENSE)）です。帰属表記に漏れがあれば Issue でお知らせください。

Licensed under **GPL-3.0** like upstream ([`LICENSE`](LICENSE)). If an attribution is missing, please open an issue.
