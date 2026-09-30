# 随 DiskWhy 打包的字体

全部是 SIL Open Font License 1.1（OFL）字体，运行时从程序内部加载，不联网。子集由 `scripts/subset-fonts.py` 生成。
OFL 允许把字体嵌入并随软件分发、允许修改（包括子集化）；修改版不得使用原字体的保留字体名（Reserved Font Name），
并且必须继续以 OFL 发布、附上许可全文。

| 文件 | 字体 | 上游 | 修改 | 许可全文 |
|---|---|---|---|---|
| `NotoSansSC-DiskWhy.woff2` | Noto Sans SC（可变字重） | [google/fonts `ofl/notosanssc`](https://github.com/google/fonts/tree/main/ofl/notosanssc) | 子集：界面文案 + 规则文案 + GB2312 一级常用字 + 常用标点；转 woff2 | `OFL-NotoSansSC.txt` |
| `DiskWhyKai-Bold.woff2` | 霞鹜文楷 LXGW WenKai Bold v1.330 | [lxgw/LxgwWenKai](https://github.com/lxgw/LxgwWenKai/releases/tag/v1.330) | 子集：只含界面文案用到的字；**按 OFL 改名为 “DiskWhy Kai”**（不再使用 LXGW / 霞鹜 名称）；转 woff2 | `OFL-LXGWWenKai.txt` |
| `InstrumentSerif-digits.woff2`、`InstrumentSerif-Italic-digits.woff2` | Instrument Serif | [google/fonts `ofl/instrumentserif`](https://github.com/google/fonts/tree/main/ofl/instrumentserif) | 子集：数字、G、B、M、K、. , / 空格 | `OFL-InstrumentSerif.txt` |
| `BricolageGrotesque-digits.woff2` | Bricolage Grotesque（可变：wght / wdth / opsz） | [google/fonts `ofl/bricolagegrotesque`](https://github.com/google/fonts/tree/main/ofl/bricolagegrotesque) | 同上 | `OFL-BricolageGrotesque.txt` |
| `Geist-latin.woff2`、`Geist-latin-ext.woff2`、`GeistMono-latin.woff2` | Geist / Geist Mono（可变字重） | [vercel/geist-font](https://github.com/vercel/geist-font)，经 `@fontsource-variable/geist` / `geist-mono` 5.3.0 | 未修改（Fontsource 按 unicode 范围切好的文件） | `OFL-Geist.txt` |

Noto Sans SC 的许可声明里保留字体名是 “Source”（来自 Adobe 的思源黑体），本子集沿用 “Noto Sans SC” 这个名字，不涉及保留名。
