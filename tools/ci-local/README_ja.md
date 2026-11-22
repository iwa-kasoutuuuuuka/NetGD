# ci-local: CI をこの PC で実行する

`.github/workflows/build.yaml` と同じ確認をローカルで行う。マージ前にブランチを検証するための道具。

```sh
sh tools/ci-local/run.sh                      # 全部 (約 30 分。fpga 約 7 分を含む)
sh tools/ci-local/run.sh --only tests         # Rust ホストテスト + gdprobe C テスト
sh tools/ci-local/run.sh --only tools,fpga    # カンマ区切りで複数
sh tools/ci-local/run.sh --worktree H:/other  # 別の worktree を検査
powershell -File tools\ci-local\run.ps1 --only builds   # PowerShell から
```

`--only` の値: `builds`(ファームのビルド) / `tests`(common/*, mcu/bootcore, mcu/fwupdater, gdprobe) / `tools`(tools/*) / `fpga`。
`--fail-fast` で最初の FAIL で止める。

## 出力

1 ステップ 1 行: `名前 OK|FAIL|SKIP 補足`。FAIL が 1 つでもあれば終了コード 1。
ログは `%TEMP%\ci-local-<日時>\<ステップ名>.log`(最後に場所を表示)。`summary.txt` に一覧。

## 実行内容

- builds: fwupdater、netgd の 6 構成(DEFMT_LOG=off 版を含む)。各 ELF の text/data を `arm-eabi-size` で表示。
  2E の `release-z,resident,sd-fast` は `tools/fwsize/check.py --gate-2e`(パッド前の使用量が 122,880 B 以下)で検査し、余裕を表示する。
- tests: `cargo test`(lzma, gamecfg, discimg, psload, bootcore, sdfast, fatmap, stackmark, pinfind, tools/sdmmc-robustness)、
  fwupdater は `--lib --target <rustc のホスト>`(自動判定)、gdprobe の C テスト(作業コピー上で実行)。
- tools: mksdcard, netgd-send, fwbackup(約 12 分、`FWBACKUP_TEST_WRAPPER=1`), fwimage, netgd-recv, netgd-menu, netgd-t2, pinfind, fwsize。
  CI の対象外だが netgd-expect も実行する。netgd-fwupd も実行する(gen_msgs.py --check は改行コードを無視する)。
- fpga: Amaranth のシミュレーションと生成 Verilog のコミット済み確認（約 7 分、うち FIFO の等価性の確認が約 4.5 分）。Python は既定で本体のチェックアウトの `fpga/amaranth/amaranth_env`（`git rev-parse --git-common-dir` の親から探すので、worktree からでも見つかる）。Verilog の確認のあとは生成ファイルを元に戻す（不一致でも作業ツリーを汚さない）。

## 注意

- `python` が DreamSDK の MSYS python のときがある。netgd-recv / netgd-menu はそれを拒否する作り(意図的)なので `py -3` で実行する。
- cargo は同じ worktree で 2 つ同時に動かさない(順番に実行する)。
- `arm-eabi-size` は PATH か `ARM_SIZE=` か DreamSDK の既定位置から探す。

## 既知の SKIP

- fpga: 本体の `fpga/amaranth/amaranth_env` が無く、amaranth も import できないと SKIP。別の venv を使うなら
  `pip install "amaranth==0.5.8" "amaranth-yosys==0.50.0.0.post123"` のうえで `CI_LOCAL_FPGA_PYTHON=<venvのpython>` を指定する
  （版が違うと生成 Verilog の先頭の yosys の版が変わり、Verilog の確認が失敗する）。
  この PC では未導入のため SKIP。
- tools-netgd-fwupd: CI に無いが実行する。`gen_msgs.py --check` は CRLF/LF の違いを無視するので、autocrlf の checkout でも通る。
