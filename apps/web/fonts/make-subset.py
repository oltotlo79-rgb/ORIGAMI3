# 同梱する日本語フォントの縮小版を作り直す。
#
# 元のNoto Sans JP(可変フォント)から、charset.txt の字だけを残し、太さの軸を
# 400〜700(既定400)へ絞り、ライセンス表示に要る名前を残したTTFを書く。
# 道具はInkscape同梱のHarfBuzzで、追加の取り込みは要らない。
#
# 使い方(リポジトリのルートから):
#   python apps/web/fonts/make-subset.py --source <元のフォント> --out <書き出し先>
#   python apps/web/fonts/make-subset.py --source <元のフォント> --out <一時ファイル> --check
#     … --check は書き出し先を上書きせず、いまの同梱物と一致するかだけ調べる
#
# 出来上がりの確かめ方は verify-subset.py（欠落0・黒画素0の字0件・送り幅0の字0件・
# 残した名前）を使う。手順の全体は README.md の「縮小版の作り直し方」に記す。

import argparse
import ctypes
import hashlib
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPOSITORY_ROOT = HERE.parents[2]
CHARSET_PATH = HERE / "charset.txt"
DEFAULT_OUT = REPOSITORY_ROOT / "apps/web/public/fonts/NotoSansJP-ORIGAMI3-subset.ttf"

# Inkscape 1.4 同梱のHarfBuzz 10.4.0。核とsubsetで別のDLLに分かれている。
HARFBUZZ_DIR = Path(r"C:\Program Files\Inkscape\bin")

# hb_subset_sets_t（HarfBuzzのsubset-input側の集合の種類）
HB_SUBSET_SETS_UNICODE = 1
HB_SUBSET_SETS_NAME_ID = 4

# 残す名前。既定は0〜6番だけなので、ライセンス表示に要る分を足す。
#   7 商標 / 8 製造者 / 9 制作者 / 11 供給元URL / 13 ライセンス説明 /
#   14 ライセンスの場所 / 16 表示用ファミリー / 17 表示用サブファミリー /
#   25 可変フォントのファミリー接頭辞
RETAINED_NAME_IDS = (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 14, 16, 17, 25)

WEIGHT_AXIS = "wght"
WEIGHT_MIN = 400.0
WEIGHT_MAX = 700.0
WEIGHT_DEFAULT = 400.0


def _tag(text: str) -> int:
    """4文字のタグをHarfBuzzの32bit値へ直す。"""
    data = text.encode("ascii")
    return (data[0] << 24) | (data[1] << 16) | (data[2] << 8) | data[3]


def _load_harfbuzz():
    if not HARFBUZZ_DIR.is_dir():
        raise SystemExit(f"HarfBuzzの置き場が見つかりません: {HARFBUZZ_DIR}")
    os.add_dll_directory(str(HARFBUZZ_DIR))
    core = ctypes.CDLL(str(HARFBUZZ_DIR / "libharfbuzz-0.dll"))
    subset = ctypes.CDLL(str(HARFBUZZ_DIR / "libharfbuzz-subset-0.dll"))

    core.hb_blob_create.restype = ctypes.c_void_p
    core.hb_blob_create.argtypes = [
        ctypes.c_char_p, ctypes.c_uint, ctypes.c_int, ctypes.c_void_p, ctypes.c_void_p
    ]
    core.hb_face_create.restype = ctypes.c_void_p
    core.hb_face_create.argtypes = [ctypes.c_void_p, ctypes.c_uint]
    core.hb_face_reference_blob.restype = ctypes.c_void_p
    core.hb_face_reference_blob.argtypes = [ctypes.c_void_p]
    core.hb_blob_get_data.restype = ctypes.POINTER(ctypes.c_char)
    core.hb_blob_get_data.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint)]
    core.hb_set_add.restype = None
    core.hb_set_add.argtypes = [ctypes.c_void_p, ctypes.c_uint]
    core.hb_set_clear.restype = None
    core.hb_set_clear.argtypes = [ctypes.c_void_p]
    for name in ("hb_blob_destroy", "hb_face_destroy"):
        getattr(core, name).restype = None
        getattr(core, name).argtypes = [ctypes.c_void_p]

    subset.hb_subset_input_create_or_fail.restype = ctypes.c_void_p
    subset.hb_subset_input_create_or_fail.argtypes = []
    subset.hb_subset_input_unicode_set.restype = ctypes.c_void_p
    subset.hb_subset_input_unicode_set.argtypes = [ctypes.c_void_p]
    subset.hb_subset_input_set.restype = ctypes.c_void_p
    subset.hb_subset_input_set.argtypes = [ctypes.c_void_p, ctypes.c_int]
    subset.hb_subset_input_set_axis_range.restype = ctypes.c_int
    subset.hb_subset_input_set_axis_range.argtypes = [
        ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint,
        ctypes.c_float, ctypes.c_float, ctypes.c_float,
    ]
    subset.hb_subset_or_fail.restype = ctypes.c_void_p
    subset.hb_subset_or_fail.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    subset.hb_subset_input_destroy.restype = None
    subset.hb_subset_input_destroy.argtypes = [ctypes.c_void_p]
    return core, subset


def build_subset(source_bytes: bytes, charset: str) -> bytes:
    core, subset = _load_harfbuzz()

    # HB_MEMORY_MODE_READONLY=0。source_bytes はこの関数の間ずっと生かしておく。
    blob = core.hb_blob_create(source_bytes, len(source_bytes), 0, None, None)
    if not blob:
        raise SystemExit("hb_blob_create が失敗しました")
    face = core.hb_face_create(blob, 0)
    if not face:
        raise SystemExit("hb_face_create が失敗しました")

    request = subset.hb_subset_input_create_or_fail()
    if not request:
        raise SystemExit("hb_subset_input_create_or_fail が失敗しました")

    unicodes = subset.hb_subset_input_unicode_set(request)
    for character in charset:
        core.hb_set_add(unicodes, ord(character))

    # 名前の既定は0〜6番だけなので、残す番号を足す(hb-subset の --name-IDs=+… と同じ)。
    name_ids = subset.hb_subset_input_set(request, HB_SUBSET_SETS_NAME_ID)
    for name_id in RETAINED_NAME_IDS:
        core.hb_set_add(name_ids, name_id)

    ok = subset.hb_subset_input_set_axis_range(
        request, face, _tag(WEIGHT_AXIS), WEIGHT_MIN, WEIGHT_MAX, WEIGHT_DEFAULT
    )
    if not ok:
        raise SystemExit("hb_subset_input_set_axis_range(wght) が失敗しました")

    result = subset.hb_subset_or_fail(face, request)
    if not result:
        raise SystemExit("hb_subset_or_fail が失敗しました")

    out_blob = core.hb_face_reference_blob(result)
    length = ctypes.c_uint(0)
    data = core.hb_blob_get_data(out_blob, ctypes.byref(length))
    out = ctypes.string_at(data, length.value)

    core.hb_blob_destroy(out_blob)
    core.hb_face_destroy(result)
    subset.hb_subset_input_destroy(request)
    core.hb_face_destroy(face)
    core.hb_blob_destroy(blob)
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", required=True, help="元のNoto Sans JP(可変フォント)")
    parser.add_argument("--out", default=str(DEFAULT_OUT), help="書き出し先のTTF")
    parser.add_argument("--charset", default=str(CHARSET_PATH), help="残す文字の一覧")
    parser.add_argument(
        "--check", action="store_true", help="書き出さず、--out といまの内容が同じかだけ調べる"
    )
    args = parser.parse_args()

    source_bytes = Path(args.source).read_bytes()
    charset = Path(args.charset).read_text(encoding="utf-8")
    print(f"元のフォント: {len(source_bytes):,} B / SHA-256 {hashlib.sha256(source_bytes).hexdigest().upper()}")
    print(f"残す文字: {len(charset):,}字")

    out_bytes = build_subset(source_bytes, charset)
    digest = hashlib.sha256(out_bytes).hexdigest().upper()
    print(f"縮小版: {len(out_bytes):,} B / SHA-256 {digest}")

    target = Path(args.out)
    if args.check:
        current = target.read_bytes()
        same = current == out_bytes
        print(
            f"いまの {target}: {len(current):,} B / "
            f"SHA-256 {hashlib.sha256(current).hexdigest().upper()} … "
            + ("一致" if same else "不一致")
        )
        return 0 if same else 1

    target.write_bytes(out_bytes)
    print(f"{target} へ書きました")
    return 0


if __name__ == "__main__":
    sys.exit(main())
