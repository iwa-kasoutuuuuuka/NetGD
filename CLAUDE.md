# NetGD — 作業ルール

NetGD は OpenGDEMU のフォーク（Dreamcast GD-ROM 置き換え基板のファームウェアと FPGA コア）。
目標・フェーズ・モデルの役割分担は `docs/PLAN.md`。

## 記録のルール（ユーザーの指示、2026-11-20）

**調査・レビュー・検証・設計判断の結果は、これまでの分も今後の分もすべて記録に残す。**

- 記録先は `docs/research/LOG.md`。項目 `R<番号>` を末尾に追記する。過去の項目は書き換えず、訂正は新しい項目として書く。
- 各項目には、担当モデル、問い、結論、根拠（`ファイル:行` / URL / コマンド出力）、信頼度（確認済み / 一次資料 / 二次資料 / 推測）、
  Opus による検証の有無、関連ファイルを書く。
- サブエージェントの報告は、要点を LOG に書き写してから会話を先に進める（報告は会話にしか残らないため）。
- 長い調査や設計は `docs/research/`・`docs/design/` に別ファイルで置き、LOG からリンクする。`docs/research/README.md` の索引も更新する。
- 判断（何を採用・却下したか、その理由）は LOG 末尾の「判断の記録」表に追記する。
- 計画が変わったら `docs/PLAN.md` も更新する。

## ビルドとテスト

- ファーム（SAM3U4E）：`cd mcu/netgd && cargo build --release`
- ファーム（SAM3U2E、GDEMU 6.0 の MCU）：`DEFMT_LOG=off cargo build --release --features sam3u2e`
  - `--target-dir` で出力先を変えないこと（`.cargo/config.toml` の `-Map=target/openmenu.map` が相対パスのため、リンクが失敗する）。
- ホストテスト：`common/discimg`・`common/lzma`・`common/gamecfg` でそれぞれ `cargo test`
- FPGA シミュ：`cd fpga/amaranth && amaranth_env/Scripts/python.exe test.py`（`test_cdda.py`・`test_timing.py`・`test_top.py` も）
- 実機での検証はユーザーの作業。実機で確かめていないことを「動く」と書かない。
