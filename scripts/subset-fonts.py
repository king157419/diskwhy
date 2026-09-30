"""生成随应用打包的字体子集（阶段 F）。

用法（需要 fontTools + brotli）：
    python scripts/subset-fonts.py <上游字体目录>

上游字体目录里要有（都来自官方发布，SIL OFL 1.1）：
    NotoSansSC-VF.ttf          google/fonts ofl/notosanssc/NotoSansSC[wght].ttf
    LXGWWenKai-Bold.ttf        lxgw/LxgwWenKai v1.330 release
    InstrumentSerif-Regular.ttf / InstrumentSerif-Italic.ttf   google/fonts ofl/instrumentserif
    Bricolage-VF.ttf           google/fonts ofl/bricolagegrotesque/BricolageGrotesque[opsz,wdth,wght].ttf

输出到 src/assets/fonts/：
    NotoSansSC-DiskWhy.woff2        界面文案 + 规则文案 + GB2312 一级常用字（3755 字）+ 常用标点
    DiskWhyKai-Bold.woff2           霞鹜文楷 Bold 的子集，只含界面文案用到的字；按 OFL 改名为 DiskWhy Kai
    InstrumentSerif-digits.woff2 / InstrumentSerif-Italic-digits.woff2   只含数字、GB、. , 空格
    BricolageGrotesque-digits.woff2 同上（保留可变轴）
Geist / Geist Mono 直接用 @fontsource-variable 包里的 latin / latin-ext woff2（见 LICENSES.md）。
"""

import glob
import os
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "src", "assets", "fonts")


def chars_in(patterns):
    s = set()
    for pat in patterns:
        for f in glob.glob(os.path.join(ROOT, pat), recursive=True):
            if os.sep + "assets" + os.sep in f:
                continue
            with open(f, encoding="utf-8") as fh:
                s.update(fh.read())
    return s


def gb2312_level1():
    out = set()
    for hi in range(0xB0, 0xD8):
        for lo in range(0xA1, 0xFF):
            try:
                out.add(bytes([hi, lo]).decode("gb2312"))
            except UnicodeDecodeError:
                pass
    return out


ASCII = {chr(c) for c in range(0x20, 0x7F)}
PUNCT = set("，。、；：？！“”‘’（）《》【】—…·～「」『』〈〉％＋－＝／")


def cjkish(ch):
    o = ord(ch)
    return o >= 0x2000 and not (0xD800 <= o <= 0xDFFF)


def run(src, dst, text, keep_names=True, flavor="woff2"):
    opts = subset.Options()
    opts.flavor = flavor
    opts.layout_features = ["*"]
    opts.name_IDs = ["*"] if keep_names else [0, 1, 2, 3, 4, 5, 6, 13, 14]
    opts.name_languages = ["*"]
    opts.hinting = False
    opts.desubroutinize = True
    font = subset.load_font(src, opts)
    sub = subset.Subsetter(opts)
    sub.populate(text="".join(sorted(text)))
    sub.subset(font)
    return font


def patched_kai(path):
    import io
    import struct

    d = bytearray(open(path, "rb").read())
    n = struct.unpack(">H", d[4:6])[0]
    for i in range(n):
        tag, _cs, off, _ln = struct.unpack(">4sIII", d[12 + 16 * i : 28 + 16 * i])
        if tag != b"cmap":
            continue
        nt = struct.unpack(">H", d[off + 2 : off + 4])[0]
        recs = [struct.unpack(">HHI", d[off + 4 + 8 * k : off + 12 + 8 * k]) for k in range(nt)]
        keep = [r for r in recs if struct.unpack(">H", d[off + r[2] : off + r[2] + 2])[0] == 12]
        # 表头原地改写：子表数变少，多出来的记录位置不再被读到
        d[off + 2 : off + 4] = struct.pack(">H", len(keep))
        for k, (p, e, o) in enumerate(keep):
            d[off + 4 + 8 * k : off + 12 + 8 * k] = struct.pack(">HHI", p, e, o)
    return io.BytesIO(bytes(d))


def add_format4(font):
    from fontTools.ttLib.tables._c_m_a_p import cmap_format_4

    cm = font["cmap"]
    best = cm.getBestCmap()
    t = cmap_format_4(4)
    t.platformID, t.platEncID, t.language = 3, 1, 0
    t.cmap = {k: v for k, v in best.items() if k <= 0xFFFF}
    cm.tables.insert(0, t)


def rename(font, family, style, ps):
    name = font["name"]
    for rid in (1, 2, 3, 4, 6, 16, 17, 21, 22):
        name.names = [r for r in name.names if r.nameID != rid]
    for rid, val in ((1, family), (2, style), (3, f"{ps};DiskWhy subset"), (4, f"{family} {style}"), (6, ps)):
        name.setName(val, rid, 3, 1, 0x409)
        name.setName(val, rid, 1, 0, 0)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    up = sys.argv[1]
    os.makedirs(OUT, exist_ok=True)

    ui = chars_in(["src/**/*.vue", "src/**/*.ts", "src/**/*.css", "index.html"])
    rules = chars_in(["rules/*.toml"])
    common = gb2312_level1()

    noto_text = {c for c in ui | rules if cjkish(c)} | common | ASCII | PUNCT
    f = run(os.path.join(up, "NotoSansSC-VF.ttf"), None, noto_text)
    f.save(os.path.join(OUT, "NotoSansSC-DiskWhy.woff2"))
    print("Noto Sans SC 字符数", len(noto_text))

    kai_text = {c for c in ui if cjkish(c)} | ASCII | PUNCT
    # 霞鹜文楷的 format 4 cmap 子表被截断了（字太多，format 4 装不下），fontTools 读它会越界：
    # 先改写 cmap 表头，只留完整的 format 12 子表，子集化后再补一个 format 4。
    f = run(patched_kai(os.path.join(up, "LXGWWenKai-Bold.ttf")), None, kai_text)
    add_format4(f)
    rename(f, "DiskWhy Kai", "Bold", "DiskWhyKai-Bold")
    f.save(os.path.join(OUT, "DiskWhyKai-Bold.woff2"))
    print("DiskWhy Kai 字符数", len(kai_text))

    digits = set("0123456789.,GBMK /")
    for src, dst in (
        ("InstrumentSerif-Regular.ttf", "InstrumentSerif-digits.woff2"),
        ("InstrumentSerif-Italic.ttf", "InstrumentSerif-Italic-digits.woff2"),
        ("Bricolage-VF.ttf", "BricolageGrotesque-digits.woff2"),
    ):
        f = run(os.path.join(up, src), None, digits)
        f.save(os.path.join(OUT, dst))

    total = 0
    for p in sorted(glob.glob(os.path.join(OUT, "*.woff2"))):
        n = os.path.getsize(p)
        total += n
        print(f"{os.path.basename(p):<40} {n / 1024:8.1f} KB")
    print(f"合计 {total / 1024 / 1024:.2f} MB")


if __name__ == "__main__":
    main()
