# 調査資料の索引

| ファイル | 内容 |
|----------|------|
| [LOG.md](LOG.md) | **すべての調査・レビュー・検証・判断の時系列の記録**（まずここを見る） |
| [stock-gdemu-flow.md](stock-gdemu-flow.md) | **純正 GDEMU の調査フロー**：写真 → プローブツール → BBA 経由の調査 → 比較 → 反映 |
| [probe-records/](probe-records/) | プローブの記録ファイルの置き場 |
| [user-tasks.html](user-tasks.html) | 実機作業ガイド（プローブ調査・導通確認・SWD プローブ準備の手順と記入表。Artifact と同じ内容） |
| [web-prep-user-tasks.md](web-prep-user-tasks.md) | 実機作業（gdprobe の実行、導通確認、SWD プローブ準備）の事前 web 調査 |
| [../../tools/gdprobe/](../../tools/gdprobe/) | プローブツール（KOS 2.3.0）。使い方は `README_ja.md` |
| [../../tools/mksdcard/](../../tools/mksdcard/) | 調査用の SD カードの中身を作るツール。使い方は `README_ja.md` |
| [../../tools/fwbackup/](../../tools/fwbackup/) | 純正ファームの読み出し可否の確認とバックアップ（読み取り専用、吸い出したファイルは非公開）。使い方は `README_ja.md` |
| [../../tools/netgd-send/](../../tools/netgd-send/) | G2 の PC 側ツール（BBA 経由の書き込みの準備と結果の確認）。使い方は `README_ja.md` |
| [gdemu6-and-parity.md](gdemu6-and-parity.md) | GDEMU 6.0 の公開情報と、本家 GDEMU との機能差分表（R7） |
| [web-findings-2026-11.md](web-findings-2026-11.md) | web 調査：openMenu の `0x52` コマンド、GDEMU.ini、AG16K、KOS g1ata、CDI のセッション（R10） |
| [../design/bba-sd-write.md](../design/bba-sd-write.md) | BBA 経由の SD 書き込みの設計（R8） |
| [../design/g1-ag16k-port.md](../design/g1-ag16k-port.md) | G1 設計：GDEMU 6.0（AG16K）への FPGA 移植（設計のみ） |
| [ag16k-config.md](ag16k-config.md) | AG16K のコンフィグ方式（PS、ビット順、ローダーの変更設計） |
| [ag16k-toolchain.md](ag16k-toolchain.md) | AG16K のビットストリームを作る道筋（Supra、オープンなツールの状況）（R69） |
| [g3-read-path.md](g3-read-path.md) | SD 読み込み経路の調査と高速化の feature（R72・R75） |
| [../design/fw-bootloader.md](../design/fw-bootloader.md) | 自己更新の安全対策 B：ロックした常駐ブート領域（R54） |
| [../design/resident-app-contract.md](../design/resident-app-contract.md) | 常駐部とアプリの引き渡しの取り決め（世代 1）（R79） |
| [../design/g1-bitstream-storage.md](../design/g1-bitstream-storage.md) | 6.0 のビットストリームの置き場所（SD の `NETGD.BIT` か埋め込みか）（R66） |
| [../boards/gdemu60/README.md](../boards/gdemu60/README.md) | GDEMU 6.0 基板の写真と部品（R6） |
| [../PLAN.md](../PLAN.md) | 計画（目標・フェーズ・役割分担） |
