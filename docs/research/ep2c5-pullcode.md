# EP2C5F256 弱プルアップ符号ビットストリーム (pullcode, k = 0..9)

GDEMU 6.0 の FPGA ダイは all-input の EP2C5F256C8 stage-0 で PS 構成でき、EP2C5
として振る舞った。次の段階として、各ボールの弱プルアップを符号化して 10 本の
ビットストリームに分け、基板側 (MCU) から各ボールがどこにつながっているかを
読み分けるための素材を CI で作る。ハードウェアではまだ一度も試していない。

## 構成

- 生成器: `tools/pinfind/mk_pullcode.py` (標準ライブラリのみ、決定的)
- 符号表: `tools/pinfind/ep2c5_pullcode.csv` (ball, pad, bank, io_type, kind, in_code, code)
- 生成ソース: `fpga/pullcode-ep2c5/pullcode.v`, `pullcode.qpf`, `k<k>/pullcode.qsf`
- ゲート: `tools/pinfind/ep2c5gate.py --pullcode k --code tools/pinfind/ep2c5_pullcode.csv --rbf ...`
- CI: `.github/workflows/fpga-pullcode-ep2c5.yaml` (k = 0..9 の matrix)。ゲートに通った
  variant だけ `pullcode-ep2c5-k<k>-rbf` を上げる。ビットストリームはコミットしない。

## 設計

- F256 の全ユーザ I/O ボール (150) と全専用クロック入力ボール (8) を入力ポート
  `p_<ボール>` にする。出力・双方向は無し、ロジックも無し。
- 構成用 / JTAG / 専用ピン (nCONFIG, nSTATUS, CONF_DONE, nCE, DCLK, DATA0, MSEL0/1,
  TCK/TMS/TDI/TDO) と電源 / NC ボールはポートにしない。
- 符号は 10 ビット中 1 が 5 個の定重み符号 (C(10,5) = 252 語)。パッド番号順にボールを
  取り、未使用の語のうちビットごとの 1 の数が最も均等になる語を選ぶ
  (同点は itertools.combinations 順で最小)。結果、どの variant でも 75 ボールが
  プルアップ ON になる。
- variant k は、符号のビット k が 1 のボールだけ `WEAK_PULL_UP_RESISTOR ON`、他の全ポートは
  OFF。バスホールドは全ポート OFF。それ以外は stage-0 と同じ
  (未使用ピン AS INPUT TRI-STATED, `CYCLONEII_RESERVE_NCEO_AFTER_CONFIGURATION` =
  USE AS REGULAR IO, INIT_DONE / DEV_CLRn / DEV_OE / CLKUSR / CRC オフ, Passive Serial,
  圧縮 ON)。
- どのボールも 5 つの variant でプルアップされ、残り 5 つではされないので、10 回の
  読み取りの ON/OFF パターンからボールを一意に決められる (配線先がプルアップに
  反応するなら)。

## 専用クロック入力は符号から外した

最初の CI run (45824026037) では 8 本のクロック入力も符号に入れた。Quartus II 13.0 SP1 は
全 variant で fit に失敗した:

```
Error (169057): Can't place I/O pin p_H1 at pin location H1 -- I/O pin uses weak pullup, which is not supported by this pin location
```

このエラーは H1 H2 J1 J2 H15 H16 J15 J16 (CLK0..CLK7、未使用時 `GND+` 表示) の、
その variant でビットが 1 のボールにちょうど出た (各ボール 10 variant 中 5 回)。
Cyclone II の専用クロック入力には弱プルアップが無いので、この 8 本は
入力ポートのまま (プルアップ OFF、符号なし) にしてある。符号表では `in_code = no`。
これらのボールはこの方法では識別できない。

## ゲート (--pullcode k)

- .pin: 符号表の全ボールが入力 `p_<ボール>`。RESERVED_INPUT の I/O は残らない。
  固定ボール (電源、構成、JTAG) は stage-0 と同じ期待表 (`ep2c5f256_balls.txt`)。
- fit.rpt: Output Pins / Bidir Pins の行なし。Input Pins と All Package Pins の両方の
  Weak Pull Up 列が符号のビット k と完全一致。Bus Hold は全ボールでオフ。
  デュアルパーパスピンとデバイスオプションの検査は stage-0 と同じ。
- `--rbf` で rbf のサイズと SHA-256 を表示し、CI はそれを gate.txt に残す。

## CI 結果 (run 45825036847, 全 variant ゲート PASS)

Quartus イメージ `ghcr.io/cepawiel/opengdemu/quartus2@sha256:d468cf04…` (stage-0 run4 と同じ)。
各 variant: ロジック 0 LE、ピン 158 / 158、出力は TDO (専用) だけ、Output / Bidir 表なし、
All Package Pins のプルアップ On は 75 ボール。ローカルでもダウンロードした
レポートに対してゲートを再実行し、全 variant PASS、rbf の SHA-256 も gate.txt と一致した。

| k | rbf サイズ (bytes) | SHA-256 |
|---|---|---|
| 0 | 48461 | `20b07d4c527c24bae7ecf00176179867587c60c7c56b136c2edc3f98b48b3bd3` |
| 1 | 48465 | `236b96853f37dd79a8949b18e2edc79f89c6659c6b23b13ce82934c14f2f0678` |
| 2 | 48479 | `cc9e6fa46d17bb8105c5d3bb08a4ecec10fa853831f86cd7e7ac60376bc822a9` |
| 3 | 48497 | `ee1fec391f944f6e5ec652c95fdb0de0cca25564943fe543aef5f0a410d3f9aa` |
| 4 | 48505 | `5edaaf2ddb2c293016ab490ddf89b610acdf06cff1a06ee6c28c25aebba695ac` |
| 5 | 48481 | `742eff4c07e39e5f707a50ba9acb635102a579c21ddfc98ba761d77af0e8580a` |
| 6 | 48497 | `4d83e01ef9dbb88da630deb9d701e15fb4c01c17f2efe3e8e15c13dad2350186` |
| 7 | 48485 | `85bcc9317c28380b8c144d4c4c2fdaa5cf1613c3e7debcb04c6a9dfe66633a0a` |
| 8 | 48483 | `e750e5e4bb419d84e9e47dec4df6d232a69d165a5258717be11451aef12ce8f6` |
| 9 | 48499 | `e11d0dd18ddc2685ff6e467262ab0c8681b188ea7155090df37098e47818ebf7` |

(参考: stage-0 は 48342 bytes、`ad0325fa…`。)

## 注意 (安全面)

- 符号に入っているボールには、v5.5 でデュアルパーパスだった C3 (ASDO)、F4 (nCSO)、
  N14 (nCEO) も含まれる。PS 構成で ASDO,nCSO は Unreserved、nCEO は通常 I/O に
  解放しているので、どれも入力ポートとして置かれ、ゲートも通っている。

- 弱プルアップ (Cyclone II で数十 kΩ 程度) は、相手が出力しているボールに対しては
  電流をほとんど流さないが、相手側の入力を High 側に寄せる。6.0 基板の配線先が
  分からないまま載せることになるので、ハードウェアで使うかどうかは別途判断する。
- 構成前と構成中は、デバイス自身の弱プルアップが全ユーザ I/O で有効 (ハードウェアの
  挙動で、ここからは変えられない)。stage-0 と同じ。
