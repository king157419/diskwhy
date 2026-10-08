# DiskWhy logo · Round 2 设计简报（Art Director）

日期：2026-10-07 · 作者：art director（Claude）· 适用：三位设计师各出一个变体
前提：保留方案 2 的创意——**一个对话气泡，里面是一张方块地图（treemap）：磁盘自己开口，告诉你每一块是什么**。用户原话：「方案 2 的思路很不错，很完美地把这个创意解释出来了，但是设计的艺术性上还是差一点。」这一轮只解决"艺术性"。

硬规则（不可商量）
- 不碰 `src/`、`src-tauri/`、`crates/`。所有产物只放在 `docs/logo/round2/<variant>/`。
- 渲染用 Playwright + 本机 Edge，解释器 `D:\conda\miniconda3\python.exe`（参考 `docs/logo/concept-2/render.py`）。**每一张渲染图自己用 Read 看过之后才能下结论**。
- 不抄任何人的标志（用户拿 MangoDisk 当"精致度"基准，不是当素材）。
- 不在图标里放文字。

---

## 1. 诊断：方案 2 为什么"艺术性差一点"

下面每一条都来自我对 `concept-2/icon.svg`、`build.py`、`png/icon@1024.png`、`icon-16/24/32.svg` 和 `_diag/c2-*.png` 的实际测量，不是印象。

### 1.1 比例：数字是随手定的，不是一个系统
- 气泡主体 832 × 704（96→928, 96→800），宽高比 1.18；左右两列 404 : 296 = 1.36；右列上下 256 : 316 = 1.23；描边 48，格缝 36（0.75 倍）。四个比例互不相关，既不是 1 : 1、2 : 1，也不是 φ。**看起来"差不多"的数字正是"不精致"的来源**——眼睛认不出秩序，就把它归类为随意。
- 格缝 36 ≠ 描边 48：两种相近但不相等的线宽并排出现，读起来像误差而不是决定。Microsoft 的原则是"avoid extremes of thick and thin"，而更隐蔽的问题是 *almost-equal*：要么完全相等，要么相差一倍以上。

### 1.2 圆角：半径互相打架，且是纯圆弧
- 外轮廓 R = 196（占主体宽 23.5%），青柠块外侧角 148，而内侧角只有 28——同一块砖上 **148 : 28 ≈ 5 : 1** 的两种圆角，青柠块因此像一个"D"而不像一块砖。纸色/珊瑚块同样是"外大内小"的不对称胶囊。
- 所有圆角都是 `A r,r` 圆弧（G1 连续，曲率不连续）。在 256 以上的尺寸，圆弧接直线的位置能看到"折痕"，这是圆角矩形和超椭圆（squircle）的核心差别：Figma 的研究结论是 squircle "registers as an entity in its own right, like the shape of a smooth pebble"，而圆弧圆角 "looks like a square with surgery performed on it"。
- 23.5% 的外半径 + 48 px 粗描边 = 贴纸/卡通。Windows 自家图标的外半径是 48 格下 2 px（≈ 4%），我们不必那么方，但 23% 已经是 iOS 图标遮罩级别（≈ 22.4%），放在 Windows 任务栏里显得"软"。

### 1.3 尾巴：一个贴上去的楔子
- `build.py` 里尾巴是多边形的第 4、5 个顶点 `(tx, y1)`、`(x0, ty)`：一个底 272、高 136 的直角三角形，粘在主体左下。主体左边是一条竖直直线一直掉到 y = 863 才用 44 px 的圆头收尾；而另外三个角是 196 的大圆角。**三个胖角 + 一个尖角 = 标准聊天气泡轮廓**（这正是"像聊天软件"的几何原因）。
- 尾巴的"膝盖"圆角 36 与主体圆角 196 之间没有任何过渡关系；尾巴斜边（约 26.6°）在 16/24 px 上是 2–3 个半透明的灰阶像素（看 `_diag/c2-16-zoom.png`、`c2-24-zoom.png`），小尺寸上读成一团污渍。
- 更好的尾巴**从砖块自己的圆角里长出来**：尾巴的一条边是砖块圆弧的切线延长，另一条边回到主体，拐点用与格缝相同的半径收圆。这样尾巴与砖块是同一笔，而不是两块零件。

### 1.4 色彩：青柠 + 珊瑚互相抢戏，珊瑚还在报警
- 青柠 `#d4f24a`（H71, S87, L62）和珊瑚 `#ff8a73`（H10, S100, L73）都是高饱和，色相差 60°，面积比 61% : 22%。两块高饱和色并排，**眼睛先落在珊瑚上**（暖色、高亮度），而珊瑚在首页的语义是"需你判断"，在图标里就是"你的磁盘有问题"。一个"安心清理"的产品，第一眼不能是警示色。
- 纸色 `#f6f1e4` 和青柠的对比度只有 1.12 : 1，在没有墨黑描边时两者会融成一片，这也是为什么方案 2 必须靠描边——**调色板是在为描边打补丁**。
- 没有"中间值"：Microsoft 要求 "Use color values in all 3 ranges, dark, medium, light"。方案 2 只有暗（墨）和亮（青柠、纸、珊瑚），缺中间值，于是深色任务栏上一旦描边消失，就只剩几块亮色互相贴着。

### 1.5 重量：描边吃掉了 29% 的面积
- 像素统计（`_diag/diag.py`）：墨黑 28.9%、青柠 43.5%、纸 12.2%、珊瑚 15.3%。**近三分之一的图标是"线"**。 16 px 上这变成：1 px 黑框 + 1 px 黑缝 + 几块 3–5 px 的色块，色块小到看不出是 treemap。
- 深色任务栏上墨黑对 `#202020` 只有 1.13 : 1——描边消失后，图标变成"一块白 + 一块粉"（看 sheet 上的 16 hinted · dark）。

### 1.6 渐变：大图有、小图无，而且用在了错的地方
- 1024 版四个形状全部加了 120° 渐变，包括**墨黑描边**（`#2a2925 → #0d0d0b`）。给黑线加渐变会让它看起来像橡胶边，珊瑚渐变在右下角（`#fb7a62`）与墨黑相接处变脏。
- 48/64/256 用带渐变的 1024 版，16/24/32 用纯色——同一图标在任务栏 24 px 与开始菜单 48 px 之间色相和"质感"不一致。规则应当是：**要么处处没有，要么一个渐变、一种用法、全部尺寸的纯色取渐变中点**。

### 1.7 光学中心：整体偏上偏左
- alpha 质心 (497, 463)，画布中心 (512, 512)：**高 49 px、左 15 px**。主体占 96→800，尾巴把包围盒拉到 907，但尾巴几乎没有质量，所以按包围盒居中的结果就是视觉偏上。256 px 以上尤其明显（主体上方留白 96，下方视觉留白 ~220）。
- 正确做法：先定尾巴，再按**质心**（不是包围盒）定位，并把质心放在几何中心略上方（约 1–2% 画布高），这是版式里"光学中心高于几何中心"的通行做法（见 §2.5）。

### 1.8 读法：第一眼是"聊天"，第二眼才是"磁盘"
- 轮廓 = 圆角矩形 + 左下尖尾，这正是微信/Messenger/LINE 的轮廓家族；内部三块色又像聊天软件里的"卡片"。
- 要让 treemap 先被读到、气泡后被读到：
  1. 砖块数量 ≥ 3 并且大小差异明显（treemap 的特征是"大小不等的矩形铺满"），
  2. 尾巴占比小、从砖块长出来而不是从框长出来，
  3. 或者干脆让气泡轮廓由砖块"拼"出来（见变体 B）。

### 1.9 小尺寸：有手工版，但没有手工"规则"
- 16 px：主体 16 × 13，尾巴斜线 2 个灰阶像素，纸色块 5 × 4、珊瑚 5 × 5 ——在深色任务栏上最亮的是那块白纸。
- 24 px：尾巴是 3 级楼梯；32 px 还可以。手工版做对了"整像素"，但没有定义"小于 N px 时去掉什么"的递减规则，三个尺寸的尾巴角度各不相同（26°、30°、34°）。

**一句话总结**：方案 2 的问题不是创意，而是**没有一个"系统"在背后**——比例、圆角、线宽、色值、尾巴各自为政，描边把它们缝在一起，于是看起来"对"但不"美"。这一轮要做的是：先定系统，再画形。

---

## 2. 研究：什么让一个 app 图标"看起来是被精心做出来的"

（来源在文末 §7。下面是从各家指南里提炼出的、对这个图标直接有用的规则。）

### 2.1 一个比喻，极少形状，轮廓优先（Microsoft / Apple）
- "Your icon should illustrate the concept of your app in a singular element using simple forms … use no more than two metaphors in a single icon."
- "Use as few shapes with as few corners as possible … A visually balanced silhouette allows good icon scalability and also avoids extremes of thick and thin shapes."
- 对我们的含义：**气泡 + treemap 是两个比喻，这是上限**。砖块 3–4 块，尾巴 1 个，没有别的。

### 2.2 网格与圆角制度（Microsoft 48 格 / Material keylines）
- Microsoft 把所有图标对齐到 48 × 48 网格，"exterior curve 2 px radius at 48, interior curve 1 px"——**外角半径 = 内角半径 × 2**，并且所有图标共用同一套半径。
- Material 的 product icon 用 keyline shapes（正方 152、圆 176、竖长 128×176、横长 176×128 于 192 格）保证不同形状的**视觉重量一致**：圆和尖形要比方形大一圈。
- 真正"精致"的来源是**同心圆角（concentric radii）**：内层形状的圆角 = 外层圆角 − 两者间距。方案 2 在青柠块外侧做到了（196 − 48 = 148），在砖块相邻处却用 28，所以一块砖两种角。

### 2.3 超椭圆 / 连续曲率（Figma, "Desperately seeking squircles"）
- "The curvature of a squircle's perimeter is continuous, whereas a rounded square's is not." 圆弧圆角在曲线与直线交界处曲率突变，大尺寸上肉眼可见。
- Figma 的 corner smoothing 参数（iOS 对应约 60%）可以在圆角矩形和 squircle 之间连续调。**只在外轮廓的大圆角上用**，小于约 2% 画布的小圆角上 smoothing 看不出来，徒增路径复杂度。
- 实现：用 Figma 式的 smoothing 贝塞尔（每个角 3 段曲线），或者 superellipse `|x/a|^n + |y/b|^n = 1`，n ≈ 4–5，采样后转成 path。`build.py` 里现成的 `_poly_rounded` 只做圆弧，需要换。

### 2.4 色彩：少色、三档明度、单色/邻近色阶（Microsoft）
- "Use color values in all 3 ranges, dark, medium, light. Make sure at least half of your icon passes a 3.0:1 contrast ratio on light and dark theme."
- "Yellow will never pass an accessible contrast ratio on light theme until it's brown." —— 青柠就是这种黄绿：在浅色任务栏永远过不了 3 : 1，**必须有一个中间明度的同色系颜色扛浅色背景**。
- 单色调色板做法："Create three colors from the same hue … adjust the light color to be brighter and the dark color to be less saturated"。即：青柠（亮）→ 苔绿（中）→ 墨（暗）就是一套完整的 monochrome palette。
- 渐变："Gradients should be subtle … limit to one or two steps … default angle 120 degrees … Avoid very tight transitions that would feel like reflections or dimension."

### 2.5 光学补偿（Marvel / logodesign.net / 字体设计常识）
- "a 120 × 120 pixel square has a larger surface area than a 120 × 120 pixel circle, so the circle must be larger to compensate." —— 圆角越大的形状越要**超出**（overshoot）网格线。
- 三角形的重心在一侧，"creates an optical illusion of being off-centre even when it isn't"——尾巴就是这样的三角形，所以要按**质心**定位，不按包围盒。
- 光学中心高于几何中心：把形体质心放在画布中心上方约 1–2%。

### 2.6 层次与阴影（Microsoft layering / Fluent / Apple 分层）
- "Icons are composed of flat objects sitting on top of the layers below it … Use as few layers as possible … shadows cast from light onto dark shapes have the best result … Inner shadows should only cast a shadow on the graphic symbol, not on to the surrounding background."
- Apple 的分层图标指引（Icon Composer）："Keep foreground shapes bold and easily recognizable … Keep the silhouette identical; change only background tone" 跨明暗变体。
- 对我们的含义：想要"高级感"，不是加高光和玻璃，而是**两层平面 + 一个方向的光 + 一个柔和阴影**，并且所有尺寸同一套规则。

### 2.7 小尺寸（Microsoft construction / Fluent）
- 必需尺寸 16/24/32/48/256；100% 缩放下标题栏/托盘 16 px、任务栏 24 px、开始磁贴 32 px。
- "As product icons scale below 48 pixels, they simplify in detail in favor of readability … use the full-fidelity icon and scale by factors of four." 
- Apple："Avoid extremely thin line weights and sharp corners, because they tend to lose detail."（方案 2 已引用，这一轮继续适用。）
- Windows 要求提供 `altform-unplated`（深色）与 `altform-lightunplated`（浅色）两套小尺寸——**我们被允许在浅/深任务栏用不同明度的同一轮廓**。这是解决"浅色任务栏青柠不可见 / 深色任务栏墨黑不可见"的正规出口。

---

## 3. 共同基础（三个变体都必须遵守）

### 3.1 画布与网格
- 母版 1024 × 1024，**64 单位网格（1 u = 16 px）**。所有坐标、半径、线宽都是整数单位（必要时允许 0.5 u）。
- 安全边距 4 u（64 px）：主体不得越过 64–960；**圆角的 overshoot 和尾巴尖允许进入边距 ≤ 1 u**。
- 主体建议占画布 80–84%（方案 2 轮廓只占画布面积 55%，主体宽 81% 但视觉偏小，因为描边吃掉了内部）。
- 光学中心：alpha 质心必须落在 (512 ± 8, 496 ± 8)（即几何中心上方约 1.5%）。用 `round2/_diag/diag.py` 检查（改路径即可）。

### 3.2 圆角制度（唯一一条"美感"硬规则）
- 一条 2 倍链：**外轮廓 R₀ = 8 u（128 px）· 次级（砖块外侧/尾巴膝）R₁ = 4 u · 砖块相邻角 R₂ = 2 u · 最小 R₃ = 1 u**。禁止出现链外的半径。
- **同心规则**：被包在外层形状里的形状，其贴外侧的圆角 = 外层半径 − 两者间距；相邻两砖之间的角用 R₂。一块砖上最多出现两种半径，并且必须符合同心规则，不得像方案 2 那样 148 : 28。
- 外轮廓的 R₀ 角用 **corner smoothing 60%**（Figma 定义）或 superellipse n = 4.5；R₁ 及以下用圆弧。
- 外半径 8 u 占主体宽约 15%——介于 Windows（4%）与 iOS（22%）之间，既不方得生硬，也不软成贴纸。

### 3.3 线、缝、尾巴
- **一种线宽**：若变体有描边，描边 = 格缝 = 2.5 u（40 px）；若无描边，格缝 = 2 u（32 px）。禁止两种相近线宽并存。
- 描边面积 ≤ 18% 的轮廓面积（方案 2 为 29%）。
- 尾巴：
  - 必须**从一块砖的 R₁ 圆角长出来**——尾巴外侧边是该圆弧的切线延长；内侧边回到主体，膝盖用 R₂ 收圆；尖端用 R₃。
  - 尾巴长度 ≤ 主体宽的 1/4，斜边角度 **45°**（小尺寸上 45° 楼梯是干净的整像素；其它角度必然产生灰阶像素）。
  - 尾巴位置任选左下或右下，但在 lockup 中若尾巴朝右下，它应指向字标（"磁盘对你说"）。

### 3.4 调色板（精确 hex，已用 `_diag/contrast.py` 核算）

| 角色 | 名称 | hex | HSL | 对 #f3f3f3 | 对 #202020 | 用途 |
|---|---|---|---|---|---|---|
| 暗 | ink 墨 | `#141412` | 60/5/7 | 16.6 | 1.13 | 描边、C 主题的暗层 |
| **中** | **moss 苔** | **`#5c7a1a`** | 79/65/29 | **4.45** | **3.30** | **全天候锚色**：无描边时扛浅色背景；青柠的同色系暗阶 |
| 亮 | lime 青柠 | `#d4f24a` | 71/87/62 | 1.14 | 12.9 | 品牌色，"可放心清理"砖 |
| 亮 | paper 纸 | `#f6f1e4` | 43/50/93 | 1.02 | 14.5 | "剩余空间"砖 |
| 中 | clay 陶 | `#8a857a` | 41/6/51 | 3.31 | 4.44 | "需你判断"砖——**取代珊瑚**，中性暖灰，不报警 |

- 两两对比：lime/moss 3.90，paper/moss 4.38，paper/clay 3.26，lime/clay 2.90，lime/paper **1.12（不得直接相邻，中间必须有缝或暗色）**，clay/moss 1.34（同上）。
- **珊瑚 `#ff8a73` 禁用。** 如果变体坚持要第三个有彩色，只能用 moss（同色系）而不是对比色。
- 这是 Microsoft 定义的 monochrome palette：lime（亮）→ moss（中）→ ink（暗）同一色相；paper/clay 是两个暖中性。整张图最多 4 色 + 可选 1 个渐变。
- 面积规则（满足"≥ 50% 面积在浅/深两种背景上都 ≥ 3 : 1"）：
  - 浅色任务栏：moss + ink（+ clay）的面积之和 ≥ 50%，**或**提供 `lightunplated` 版本把 lime 换成 moss、paper 换成 clay（轮廓不变，只换明度）。
  - 深色任务栏：lime + paper（+ clay）≥ 50%。
- 渐变（可选，每个变体最多一个）：只允许在 **lime 砖**上，120°，光来自左上，`#e0f766 → #c8e93a`（中点就是 `#d4f24a`）。≥ 32 px 用渐变，16/24 用纯色 `#d4f24a`，这样小图与大图色相一致。禁止在墨黑、纸、陶、描边上加渐变。
- 主题映射（标题栏 16 px 单色/双色版，不是任务栏图标）：
  - A 浅色：暗层 `#1c1b18`，亮层 `#2f7d5b`（A 的绿，对 A 背景 4.42）。
  - B 深色：亮层 `#f3ece2`，强调 `#e4bc70`（对 B 背景 9.9）。
  - C 卡通：ink + lime，允许 1 px 描边 + 1 px 硬偏移阴影（C 的语言）。
  - 三个主题共用同一轮廓，只换色。

### 3.5 小尺寸规则（16 / 24 / 32 都单独手绘，整像素）
| | 16 | 24 | 32 |
|---|---|---|---|
| 主体占位 | 16 × 14（或 15 × 13 + 1 px 尾） | 22 × 19 | 30 × 26 |
| 外圆角 | 2 px（2 像素斜切或 1+1 阶） | 3 px | 4 px |
| 描边（若有） | 1 px | 1 px | 2 px |
| 格缝 | 1 px | 1 px | 2 px |
| 砖块数 | ≤ 3，最小砖 3 × 3 | ≤ 4，最小砖 4 × 4 | ≤ 4，最小砖 5 × 5 |
| 尾巴 | 45°，2 阶（2 × 2） | 45°，3 阶 | 45°，4 阶 |
| 渐变 | 无 | 无 | 允许（lime 砖） |
- 轴对齐边一律落在整像素上，不允许半透明边；唯一允许的抗锯齿是圆角像素和 45° 楼梯的角点。
- 必须出 `*-light.svg`（浅色任务栏）与 `*-dark.svg`（深色任务栏）两套小尺寸，轮廓完全一致。
- 验收：在 `#f3f3f3` 与 `#202020` 上各看一次 1× 像素，并看 ×4 放大。sheet 上必须包含"naive downscale"对照列，证明手绘版确实更好。

### 3.6 禁止清单
- 文字；珊瑚/红/橙；玻璃、高光、反光、斜面；超过一个渐变；两种相近线宽；链外半径；非 45° 的尾巴；按包围盒居中；大图小图不同色相；任何现成品牌的轮廓。

---

## 4. 三个变体方向（各一位设计师，互不串门）

三者共享 §3，差异只在"怎样让气泡与砖块成为一体"。每个变体先出 1024 母版，再出 16/24/32 手绘版、浅/深两套、三主题 16 px 标题栏版、lockup、sheet.png 和 NOTES.md（说明用了哪些规则、测了什么）。

### 变体 A ·「留白气泡」Quiet Grid —— 精修扁平，负形代替描边
- **关键一招**：没有描边。砖块之间是 2 u 的**透明缝**（露出任务栏背景），气泡轮廓只由砖块的外边缘"合"出来。至少一块大砖是 moss（锚色），让浅色任务栏上有形；lime 砖最大，paper/clay 次之。尾巴从 moss 或 lime 砖的 R₁ 角长出。
- 为什么能更艺术：去掉 29% 的黑线后，颜色本身承担结构；留白缝让 treemap 的"铺满感"一眼可见，气泡成为第二读。
- 建议砖块布局：3 块，比例取 **φ 链**——主体宽 52 u：左列 32 u / 右列 18 u（+2 u 缝），右列上下 20 u / 10 u（2 : 1）。
- 要避免：缝细于 1 px（16 px 上缝 = 1 px）；四块以上；lime 与 paper 直接相邻（1.12 : 1 会融掉，必须隔 moss 或缝）；把缝画成浅灰色线（那就是描边，不是留白）。
- 浅色任务栏小尺寸：moss 砖面积 ≥ 50%，或 lightunplated 版把 lime 砖降为 moss、paper 砖降为 clay。

### 变体 B ·「长出来的尾巴」Grown Tail —— 一个连续形体，砖块本身就是气泡
- **关键一招**：lime 砖就是气泡主体（最大、带 R₀ squircle 外角），另外两三块砖以"切口"的方式**嵌进** lime 砖的一侧（像 treemap 把一块大砖切出小格），尾巴是 lime 砖左下 R₁ 角的切线延长。整个图标是**一条外轮廓、一个颜色区域里挖出几块**，不存在"框"这个层。
- 色彩：单色系为主——lime 主体，嵌入砖用 moss 和 paper（或 moss 与 clay），整张 3 色；深色任务栏靠 lime，浅色任务栏靠 moss 砖 + lightunplated 版。
- 为什么能更艺术：这是"少形状"原则的极限做法；曲率连续的单一外形在 256 px 以上会像一枚打磨过的物件；尾巴与主体同一笔，彻底摆脱"贴上去的楔子"。
- 要避免：嵌入砖的角用了链外半径；嵌入砖之间用描边分隔（只能用 2 u 的 lime 本色"桥"或 moss 色块分隔）；尾巴独立成三角；主体被切得像"田"字（那就是窗口图标）。
- 16 px：lime 主体 + 1 块 moss 砖 + 1 块 paper 砖即可；尾巴 2 × 2 楼梯。

### 变体 C ·「叠层」Layered Plate —— 克制的立体感，一层板、一层砖、一个光
- **关键一招**：两层。底层是气泡形"板"（含尾巴，R₀ squircle，颜色 moss 或 ink），上层是 treemap 砖块（lime / paper / clay），砖块比板**内缩 2.5 u**（同心规则自动给出砖块圆角 = 8 − 2.5 = 5.5 u → 取 R₁ + 1.5；允许本变体用 5.5 u 作为唯一例外，并在 NOTES 写明）。光从左上来：只有 lime 砖有那个唯一的 120° 渐变；砖块对板投一个**柔和内阴影**（Microsoft："shadows cast from light onto dark shapes have the best result"），偏移 (0, 1 u)，模糊 1 u，不透明度 ≤ 25%，阴影不得溢出板外。
- 层次用来说"砖块是放在气泡里的东西"，而不是用来炫技；所有尺寸用同一套（≥ 32 有渐变 + 阴影，16/24 用纯色 + 1 px 硬偏移或无阴影）。
- 为什么能更艺术：板给了深色任务栏一个完整轮廓，砖给了浅色任务栏对比；一个方向的光让三块砖有"物"的感觉，这是 MangoDisk 这类图标"精致"的真正来源——不是渐变多，而是光只有一个方向、所有尺寸一致。
- 要避免：玻璃/高光/外发光；给板加渐变；阴影偏移超过 1 u；C 主题的硬阴影与本变体的柔阴影混用（C 主题标题栏版单独做硬阴影）；板与砖之间出现第三种线宽。

### 三个变体共同的"读法"验收
1. 3 米测试：256 px 在 1.5 米外看，先看到的是"几块大小不等的砖"而不是"聊天气泡"。
2. 16 px 在深色任务栏上，必须能数出 ≥ 2 块砖；在浅色任务栏上，轮廓必须完整。
3. 与微信/Messenger/LINE 的图标放在同一行，不能被误认为同类。
4. 与 MangoDisk 放在同一行，精致度不输，轮廓不相似。

---

## 5. 交付清单（每个变体）
```
docs/logo/round2/<a-quiet-grid|b-grown-tail|c-layered-plate>/
  build.py          # 参数化生成所有 SVG（可复用 concept-2/build.py 的骨架，但圆角函数要换成 smoothing/superellipse）
  render.py         # Playwright + Edge → png/ + sheet.png（复用 concept-2/render.py）
  icon.svg          # 1024 母版
  icon-16.svg icon-24.svg icon-32.svg           # 手绘整像素（深色任务栏用）
  icon-16-light.svg icon-24-light.svg icon-32-light.svg   # 浅色任务栏用，同轮廓换明度
  lockup.svg lockup-dark.svg lockup-A.svg lockup-B.svg lockup-C.svg
  png/              # 16 24 32 48 64 256 1024，浅/深
  sheet.png         # 必含：两种任务栏 1× 行、×4 放大（含 naive downscale 对照）、三主题标题栏、lockup、质心叠加图
  NOTES.md          # 一句话创意 · 用了哪些 §3 规则 · 测量结果（质心、面积比、对比度）· 迭代记录
```
对比图：`docs/logo/round2/compare.png`，由 `round2/_compare/build_compare.py` 生成（照 `docs/logo/_compare/build_compare.py` 改路径），三列并排，含 256 / 32 / 16 / 标题栏 C 与 A。

工具（已在 `round2/_diag/`）：`diag.py`（质心、面积比、小尺寸像素放大）、`contrast.py`（对比度，可传额外 hex）。

---

## 6. 给设计师的最后三句话
1. 先把 §3 的数字画成网格，再往里放形；**不允许"看着差不多"的数值**。
2. 每一张渲染图自己看一遍（Read PNG），并在 NOTES 里写下看到了什么、改了什么。
3. 艺术性来自克制和一致：一种线宽、一条半径链、一个色相家族、一个光向、一个尾巴角度。

---

## 7. 来源
- Microsoft, *Design guidelines for Windows app icons*（比喻数量、48 格 2 px / 1 px 圆角、轮廓、三档明度、3 : 1 规则、120° 渐变、分层与阴影）— https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-design
- Microsoft, *Construct your Windows app's icon*（16/24/32/48/256 必需尺寸、各尺寸用途、`altform-unplated` / `altform-lightunplated` 浅深两套）— https://learn.microsoft.com/en-us/windows/apps/design/iconography/app-icon-construction
- Microsoft Fluent 2, *Iconography*（<48 px 简化细节、按 4 倍缩放、单色谨慎）— https://fluent2.microsoft.design/iconography
- Apple HIG, *App icons*（简单、单一焦点、避免细线和尖角、避免文字；分层图标保持轮廓一致）— https://developer.apple.com/design/human-interface-guidelines/app-icons （本日页面为 JS 渲染未能抓取正文，引用的句子沿用 concept-2/NOTES.md 已核对过的原文）
- Apple, *Icon Composer* / 分层图标指引摘要（前景保持粗壮可辨、轮廓跨明暗不变）— https://www.developer.apple.com/icon-composer/ ；https://bitrig.com/blog/liquid-glass-app-icon
- Figma, *Desperately seeking squircles*（曲率连续、corner smoothing 参数、iOS ≈ 60%）— https://www.figma.com/blog/desperately-seeking-squircles/
- Material Design, *Product icons*（keyline shapes 152/176/128×176、一致圆角、视觉重量补偿）— https://m2.material.io/design/iconography/product-icons.html ；https://m1.material.io/style/icons.html
- Marvel, *Optical adjustment: logic vs designers*（圆需大于方、三角形重心偏移、overshoot）— https://marvelapp.com/blog/optical-adjustment-logic-vs-designers/
- logodesign.net, *Optical adjustments in logo design* — https://www.logodesign.net/blog/optical-adjustments-in-logo-design/
- Microsoft Design, *Iconic icons: designing the world of Windows*（深度与颜色作为次级线索、系统一致性）— https://medium.com/microsoft-design/iconic-icons-designing-the-world-of-windows-5e70e25e5416
- NN/g, *Icon usability*（一眼读不懂的图标就是噪音；沿用 concept-2 的引用）— https://www.nngroup.com/articles/icon-usability/
- 本地测量：`docs/logo/round2/_diag/diag.py`（质心、面积比、像素放大）、`docs/logo/round2/_diag/contrast.py`（WCAG 对比度）
