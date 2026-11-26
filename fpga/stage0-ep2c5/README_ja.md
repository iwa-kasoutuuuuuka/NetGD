# stage-0（EP2C5F256C8、何も駆動しない構成）

GDEMU 6.0 の FPGA（刻印 AG16KDDF256）は、ピン探しの実験で Cyclone II EP2C5 の本物のストリームを CRC つきで受け付けた（sweep1〜6、`docs/research/ag16k-ps-runs-20261124.md`）。
このフォルダは、その仮説のもとで **CONF_DONE を初めて上げるための最小構成** を作る。

- 設計は空（ポートもロジックも無い）。
- 未使用ピンはすべて「入力・トライステート」。古い Quartus の既定（出力で GND を駆動）にはしない。
- nCEO・ASDO・nCSO も、コンフィグ後はプログラミング用ピンとして残さない。INIT_DONE・DEV_CLRn・DEV_OE・CRC_ERROR は使わない。
- `check_pins.py` が fitter の `.pin` を検査する。電源・GND、`RESERVED_INPUT`、専用のコンフィグ / JTAG ピン以外の行（`GND+` やユーザーの信号）があれば失敗。

ビルドは GitHub Actions（`.github/workflows/stage0-ep2c5.yaml`、Quartus II 13.0 SP1 の公開イメージ）。
出力の `stage0.rbf` と `.pin`・レポートは、アーティファクト `stage0-ep2c5` から取る。`stage0.rbf` はコミットしない。

**未実行・未検証。** これはビルドと検査だけで、実機に流すかどうか・いつ流すかは別に決める。
流す前に少なくとも次を確かめる。

1. sweep7（本物のストリームを最後のフレームの手前で打ち切る）が予測どおりで、チップが EP2C5 として全フレームを検査していること。
2. `stage0.rbf` が OpenGDEMU.rbf と同じ形式（ヘッダ、圧縮）であること。CI の要約に先頭 48 バイトを並べて出す。
3. 流すときは、CONF_DONE を確かめたら短時間で nCONFIG を Low に戻す（ピン探しファームと同じ安全の約束）。

チップが EP2C5 でなかった場合（EP2C8 など）は、CONF_DONE が上がらないだけで、ピンは動かない見込み。ただしこれは推測。
