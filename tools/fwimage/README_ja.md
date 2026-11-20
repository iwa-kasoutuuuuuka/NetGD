# tools/fwimage

NetGD の自己更新（`0xF9`）用の更新イメージ（"OGFW"）を作る・調べるホスト側ツール。Python 標準ライブラリのみ。
形式は [FORMAT.md](FORMAT.md)、更新側の実装は `mcu/fwupdater`。

```
# ファームのビルド（CLAUDE.md の手順どおり。ビルドのたびに ELF は上書きされる）
cd mcu/opengdemu
cargo build --release                                              # 4E
DEFMT_LOG=off cargo build --release --features sam3u2e             # 2E

python tools/fwimage/fwimage.py build --elf mcu/opengdemu/target/thumbv7m-none-eabi/release/opengdemu \
       --target 2e -o fw_2e.ogfw [--image-version N]
python tools/fwimage/fwimage.py inspect fw_2e.ogfw [--target 2e|4e]
```

- `build`：ELF の PT_LOAD（LMA）からフラッシュ範囲を取り出す。プレーン0（`0x80000`〜）はコードイメージ、プレーン1（`0x100000`〜、4E のみ）はビットストリームイメージ。
  隙間と 512 バイト境界までの端数は 0xFF。ヘッダにターゲットの CHIPID_CIDR、長さ、CRC-32 を入れる。
  作る前にベクタ表（SP が対象の SRAM0 内、Reset/NMI/HardFault が奇数でイメージ内）とヘッダを更新側と同じ規則で検査する。
  2E 向けに 4E の ELF（プレーン1あり）を渡すと拒否する。
- `inspect`：ヘッダ CRC・ペイロード CRC・ベクタ表を検査し、どの型番向けかを表示する。`--target` で別の型番に対する検査もできる（食い違えば BadTarget）。
- 送信側（BBA 経由）はまだ無い。`fwimage.iter_images(data)` が CRC 検査済みのイメージ（`Image.header` / `.payload`）を返す。
  送る順序は FORMAT.md の「Sending」。

## テスト

```
python -m unittest discover -s tools/fwimage -v
# 実 ELF でも検査する場合（ビルドごとに別名で保存しておく）
FWIMAGE_ELF_4E=... FWIMAGE_ELF_2E=... python -m unittest discover -s tools/fwimage -v
cd mcu/fwupdater && cargo test --lib --target x86_64-pc-windows-msvc
```

`vectors/*.txt`（CRC、ベクタ表、ヘッダの共有ケース）は Rust 側（`mcu/fwupdater/src/logic.rs`）と Python 側の両方が読む。
再生成は `python tools/fwimage/gen_vectors.py`（期待値は手書き）。

## 注意

- **実機では未検証。** 更新プログラムとこのイメージ形式を実機で動かした記録はまだ無い。
- 更新中に電源が落ちる・ホストが消えると、ページ0を書き換えるまでの間は SWD でしか戻せない（FORMAT.md 末尾）。
  常駐ブート領域（LOG R37 の対策 B）が入るまで、2E での更新は SWD の手段（Pico など）を用意してから行う。
