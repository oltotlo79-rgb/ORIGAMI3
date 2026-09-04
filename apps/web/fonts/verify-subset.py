# 作り直した縮小版フォントを実測で確かめる。
#
#   1. 元のフォントが charset.txt の字を全て持っていること(持っていない字は止めて報告する)
#   2. 縮小版が charset.txt の字を1つ残らず収録していること(欠落0)
#   3. 1字ずつ32pxへ実描画して、空白字U+3000を除く黒画素0の字が0件、送り幅0の字が0件
#   4. name表に0・7・9・13・14番が残っていること(OFL条件2の著作権表示とライセンス本文)
#
# 使い方(リポジトリのルートから):
#   python apps/web/fonts/verify-subset.py --source <元のフォント>
#
# 描画には freetype-py を使う。無ければ `pip install --user freetype-py` で入れる
# (Inkscape同梱の libfreetype-6.dll を ctypes で直に叩くと、FT_FaceRec の並びを
#  手で書き写すことになり、ずれても静かに間違った結果が出るため使わない)。

import argparse
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPOSITORY_ROOT = HERE.parents[2]
CHARSET_PATH = HERE / "charset.txt"
DEFAULT_FONT = REPOSITORY_ROOT / "apps/web/public/fonts/NotoSansJP-ORIGAMI3-subset.ttf"

IDEOGRAPHIC_SPACE = "　"
RENDER_PIXELS = 32

# OFL条件2のために必ず残す名前。README「利用条件」と同じ並び。
REQUIRED_NAME_IDS = (0, 7, 9, 13, 14)
NAME_LABELS = {
    0: "著作権",
    1: "ファミリー",
    2: "サブファミリー",
    3: "識別子",
    4: "表示名",
    5: "版",
    6: "PostScript名",
    7: "商標",
    8: "製造者",
    9: "制作者",
    11: "供給元URL",
    13: "ライセンス説明",
    14: "ライセンスの場所",
    16: "表示用ファミリー",
    17: "表示用サブファミリー",
    25: "可変フォントのファミリー接頭辞",
}


def _tables(font: bytes) -> dict:
    count = struct.unpack_from(">H", font, 4)[0]
    found = {}
    for index in range(count):
        record = 12 + 16 * index
        tag = font[record:record + 4].decode("latin1")
        offset, length = struct.unpack_from(">II", font, record + 8)
        found[tag] = (offset, length)
    return found


def codepoints(font: bytes) -> set:
    """cmapの形式4と形式12を読んで、収録しているUnicodeの集合を返す。"""
    cmap_offset = _tables(font)["cmap"][0]
    subtable_count = struct.unpack_from(">H", font, cmap_offset + 2)[0]
    covered = set()
    for index in range(subtable_count):
        record = cmap_offset + 4 + 8 * index
        platform, encoding = struct.unpack_from(">HH", font, record)
        offset = cmap_offset + struct.unpack_from(">I", font, record + 4)[0]
        fmt = struct.unpack_from(">H", font, offset)[0]
        if fmt == 4 and platform == 3 and encoding == 1:
            segments = struct.unpack_from(">H", font, offset + 6)[0] // 2
            end_base = offset + 14
            start_base = end_base + segments * 2 + 2
            delta_base = start_base + segments * 2
            range_base = delta_base + segments * 2
            for segment in range(segments):
                end = struct.unpack_from(">H", font, end_base + segment * 2)[0]
                start = struct.unpack_from(">H", font, start_base + segment * 2)[0]
                if start == 0xFFFF:
                    continue
                delta = struct.unpack_from(">h", font, delta_base + segment * 2)[0]
                range_offset = struct.unpack_from(">H", font, range_base + segment * 2)[0]
                for code in range(start, end + 1):
                    if range_offset == 0:
                        glyph = (code + delta) & 0xFFFF
                    else:
                        at = range_base + segment * 2 + range_offset + (code - start) * 2
                        if at + 2 > len(font):
                            continue
                        glyph = struct.unpack_from(">H", font, at)[0]
                        if glyph:
                            glyph = (glyph + delta) & 0xFFFF
                    if glyph:
                        covered.add(code)
        elif fmt == 12 and platform == 3 and encoding == 10:
            groups = struct.unpack_from(">I", font, offset + 12)[0]
            for group in range(groups):
                at = offset + 16 + group * 12
                start, end, glyph = struct.unpack_from(">III", font, at)
                if glyph:
                    covered.update(range(start, end + 1))
    return covered


def name_ids(font: bytes) -> dict:
    offset = _tables(font)["name"][0]
    count, storage = struct.unpack_from(">HH", font, offset + 2)
    found = {}
    for index in range(count):
        record = offset + 6 + 12 * index
        platform, encoding, language, name_id, length, at = struct.unpack_from(">HHHHHH", font, record)
        raw = font[offset + storage + at: offset + storage + at + length]
        try:
            text = raw.decode("utf-16-be") if platform == 3 else raw.decode("latin1")
        except UnicodeDecodeError:
            text = repr(raw)
        found.setdefault(name_id, text)
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--font", default=str(DEFAULT_FONT), help="調べる縮小版のTTF")
    parser.add_argument("--source", default=None, help="元のフォント(あれば収録も調べる)")
    parser.add_argument("--charset", default=str(CHARSET_PATH))
    args = parser.parse_args()

    charset = Path(args.charset).read_text(encoding="utf-8")
    font_path = Path(args.font)
    font = font_path.read_bytes()
    failures = []

    print(f"charset: {len(charset):,}字")

    if args.source:
        source_covered = codepoints(Path(args.source).read_bytes())
        absent = [one for one in charset if ord(one) not in source_covered]
        print(f"1. 元のフォントに無い字: {len(absent)}件" + (f" 「{''.join(absent)}」" if absent else ""))
        if absent:
            failures.append("元のフォントに無い字がcharsetにある(図へ置き換えるか、字をやめる判断が要る)")

    covered = codepoints(font)
    missing = [one for one in charset if ord(one) not in covered]
    print(f"2. 収録: {len(charset) - len(missing)}/{len(charset)} 欠落{len(missing)}件"
          + (f" 「{''.join(missing)}」" if missing else ""))
    if missing:
        failures.append("縮小版に入っていない字がある")

    import freetype

    face = freetype.Face(str(font_path))
    face.set_pixel_sizes(0, RENDER_PIXELS)
    blank = []
    zero_advance = []
    for one in charset:
        face.load_char(one, freetype.FT_LOAD_RENDER)
        bitmap = face.glyph.bitmap
        ink = sum(1 for value in bitmap.buffer if value)
        if ink == 0 and one != IDEOGRAPHIC_SPACE:
            blank.append(one)
        if face.glyph.advance.x == 0:
            zero_advance.append(one)
    print(f"3. {RENDER_PIXELS}pxで実描画: 黒画素0の字 {len(blank)}件"
          + (f" 「{''.join(blank)}」" if blank else "")
          + f" / 送り幅0の字 {len(zero_advance)}件"
          + (f" 「{''.join(zero_advance)}」" if zero_advance else ""))
    if blank:
        failures.append("黒画素0の字がある")
    if zero_advance:
        failures.append("送り幅0の字がある")

    names = name_ids(font)
    kept = sorted(names)
    print(f"4. 残った名前: {kept}")
    for name_id in REQUIRED_NAME_IDS:
        label = NAME_LABELS.get(name_id, str(name_id))
        if name_id in names:
            print(f"   {name_id}番({label}): {names[name_id][:90]}")
        else:
            failures.append(f"name表の{name_id}番({label})が消えている")

    print(f"\n{font_path}: {len(font):,} B")
    if failures:
        for one in failures:
            print(f"不合格: {one}")
        return 1
    print("すべて合格")
    return 0


if __name__ == "__main__":
    sys.exit(main())
