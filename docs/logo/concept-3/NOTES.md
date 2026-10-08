# Concept 3 · “Di” 字标（lettermark）

**一句话（给不懂设计的人）**：一个很粗的 D，中间掏空成一个 i。i 代表“给你说明白”，i 头上那一小片就是容量柱最上面那截“可放心清理”。D+i 正好也是 DiskWhy 的头两个字母。

## 我用的规则（附来源）

1. **一个核心比喻，最多两个，不加装饰**：icon 只讲一件事（这里是“磁盘 + 说明”），去掉所有不支撑比喻的元素。
   来源：Microsoft, *Design guidelines for Windows app icons*: “use no more than two metaphors in a single icon”。
   https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-design
2. **轮廓要有辨识度，形状和转角越少越好，避免极粗极细对比**：整个标只有 tile、D、两个镂空块。
   来源：同上（Shape › Silhouette）。
3. **外圆角 / 内圆角按网格比例**：48 网格下外角 2px、内角 1px → 1024 上外角 ≈44、内角 ≈20–24。D 的左侧外角 44，i 的镂空内角 20–24。
   来源：同上（The grid and rounded corners）。
4. **小尺寸单独简化，像素对齐**：Win32 至少要 16/24/32/48/256；标题栏/托盘 16px、任务栏 24px（100% 缩放）。16 和 24 单独手画在像素网格上（`icon-16.svg`、`icon-24.svg`），16 去掉硬投影。
   来源：Microsoft, *Construct your Windows app's icon*（尺寸表 + “at the bare minimum: 16x16, 24x24, 32x32, 48x48, and 256x256”）。
   https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction
5. **浅色和深色任务栏都要可见：用深/中/浅三档明度，至少一半面积对两种背景达到 3:1**：墨黑描边 + 墨黑字母管浅色任务栏，青柠底管深色任务栏。不靠颜色单独传达意思（单色版照样成立）。
   来源：Microsoft, app-icon-design › Color contrast。
6. **颜色少、不用渐变和反光**：主图只有 2 色（青柠 #d4f24c + 墨 #141412），主题配色版最多 3 色。
   来源：Microsoft, app-icon-design › Color and gradients（“treatments to colors should be minimized”）。
7. **能单色、能极小尺寸复刻**：`icon-mono.svg` 是纯单色真镂空版。
   来源：Paul Rand, *Logos, Flags, and Escutcheons* (1991)：logo 必须 “reproducible in one color and in exceedingly small sizes”。
   https://www.paulrand.design/writing/articles/1991-logos-flags-and-escutcheons.html
8. **简单、独特、好记、不追潮流**：几何化的 D + 一个“巧思”的负空间，而不是插画。
   来源：同上（Rand：logo 的作用是识别，手段是简单）。
9. **一般不放文字，除非必要**：Microsoft 建议 icon 里避免字母。这一条本方案是**有意违反**的（任务角度要求字标），所以用“i = 信息”这个通用符号把字母变成比喻，而不是单纯印首字母。
   来源：Microsoft, app-icon-design › Representation。

## 文件

| 文件 | 用途 |
|---|---|
| `icon.svg` | 主 app icon（1024，C 主题贴纸风：青柠底 + 墨黑描边 + 硬投影 + 墨黑 D） |
| `icon-24.svg` / `icon-16.svg` | 手调像素版（任务栏 24px、标题栏/右键菜单 16px） |
| `icon-mono.svg` | 单色真镂空版 |
| `icon-theme-A.svg` / `icon-theme-B.svg` / `icon-theme-ink.svg` | 浅色 / 深色 / 墨底青柠 三种配色 |
| `mark-16-A.svg` / `mark-16-B.svg` / `mark-16-C.svg` | 标题栏用的裸 D（13×16，像素对齐，按主题上色） |
| `lockup-light.svg` / `lockup-dark.svg` | 横排组合：icon + “DiskWhy”（Geist Bold 转曲线，i 的圆点换成同款扁片） |
| `sheet.png` | 预览总表 |
| `png/` | 各尺寸渲染 |
| `build/` | 生成脚本（`build.py`，Playwright + 本机 Edge 渲染）；`iter/r1`、`iter/r2` 是前两轮草稿 |

字体：Geist（SIL OFL 1.1，项目已打包于 `src/assets/fonts`，许可见 `OFL-Geist.txt`），lockup 里已转成路径，不依赖字体安装。

## 迭代记录
- r1：方角大圆角 D + 矮胖 i，5 种配色。问题：D 像圆角方块不像 D；i 太矮像方块；16px 下点和竖粘在一起。
- r2：改成真正的半圆碗 D，i 拉高，左右墙厚平衡；试了“点歪着飞出去”的版本——大图有趣，小图像重音符号 í，放弃，回到端正的点（更耐看）。比较了墨底 / 青柠贴纸 / 无底板三种，贴纸版在两种任务栏上都最醒目。
- r3：定稿几何；字母缩到 90% 让四周留白均匀；点的圆角统一；手画 16/24；做 lockup（修了字形计数器被填实的 bug，点加粗描边、放大）。
