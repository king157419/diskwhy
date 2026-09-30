# DiskWhy 第二版设计 · 研究笔记（2026-09-29）

## 1. 为什么 AI 生成的界面长一个样，怎么去掉
- [SmoothUI · AI Design Slop](https://smoothui.dev/blog/ai-design-slop)：列出的典型特征是紫青渐变、玻璃拟态、一排六张一样的卡（图标+标题+两行字）、每个 hover 都弹一下。借鉴：把这几条当成禁用清单。
- [DEV · The Purple Gradient Problem](https://dev.to/james_anderson_h/the-purple-gradient-problem-why-ai-ui-all-looks-alike-and-how-to-fix-it-3j65)、[Drawbackwards](https://www.drawbackwards.com/blog/ai-generated-ui-making-every-product-look-the-same)：根本原因是没有设计约束时模型给出训练数据的"平均值"。借鉴：先定死 token（色板、字号阶梯、圆角），再画页面。
- [fajarhide/anti-slop](https://github.com/fajarhide/anti-slop)：不规定风格，只要求"每个技巧都要说出理由"，并禁止编造数据。借鉴：稿子里的数字、路径、规则后果全部取自 `rules/*.toml` 真实文案。
- [Anthropic frontend-design skill](https://github.com/anthropics/skills)（本次已加载）：先选一个极端方向并做到位；不要默认字体；深浅主题各自有态度。

**本次采用的 8 条去 AI 感手法**
1. **非对称构图**：首页左重右轻（大数字 + 竖向容量柱），不再居中对称套圆圈。
2. **有性格的数字字体**：大数字用展示字体（A 衬线 / B 细体 / C 窄粗体），小数字统一用等宽数字（tabular-nums）。
3. **分隔线代替卡片**：清理页没有一张圆角卡，靠 1px 细线、缩进和列对齐分组。
4. **层次靠字重和颜色，不靠边框和底色**：名字 500、路径等宽灰色、大小右对齐 500。
5. **真实内容密度**：每行给出真实路径（`%LOCALAPPDATA%\npm-cache`）和真实后果，行高 52px，一屏看 10+ 行。
6. **克制的圆角**：A 6px / B 8px；只有 C 主题用大圆角 + 粗描边，并且是"故意的"。
7. **标注贴着数据走**：容量柱每段旁边直接写名字和大小，不做图例。
8. **一屏一个主角**：每页只有一个实心主按钮；次要动作是纯文字链接。

## 2. 工具型桌面应用怎么处理"一个大数字 + 一个动作"和"列表 + 复选"
- [Linear · How we redesigned the UI](https://linear.app/now/how-we-redesigned-the-linear-ui)：用 LCH 只靠 3 个变量（底色、强调色、对比度）生成整套主题；标题换 Inter Display 增加表情；减少彩色"铬"。借鉴：三主题都用"底色/强调/对比"推导，强调色只在可点和"可放心清理"上出现。
- [Raycast 设计分析](https://github.com/VoltAgent/awesome-design-md/blob/main/design-md/raycast/DESIGN.md)：行紧凑统一、选中态高对比、一个暖色强调、细线描边代替阴影。借鉴：清理列表的行规格、左侧分类的选中态（左竖线 + 加粗，不加底色卡）。
- [DaisyDisk · Understanding sunburst](https://daisydiskapp.com/guide/2/en/UnderstandingSunburst/)：左图右表、按大小排序、小对象合并成"更小的对象"。借鉴：探索页"方块图 + 列表并排、联动"，小于 1% 的合并成"其他"。
- [Klarity · Treemap vs Sunburst](https://www.klaritydisk.com/blog/treemap-icicle-vs-sunburst)：矩形布局标签可水平书写、按面积比较更准。借鉴：探索页用方块图（treemap），首页磁盘状态用**竖向堆叠柱**而不是圆环——圆环的四段标签只能放图例或斜放，正是用户看不懂的原因。
- [CleanMyMac 2024 改版](https://9to5mac.com/2024/10/16/macpaw-releases-major-update-to-cleanmymac-with-fresh-design-and-new-features/)：扫描后一个结果 + 一个大按钮，模块压到 6 个，文案去技术化。借鉴：首页可点 ≤3。
- [Things 3（两届 Apple Design Award）](https://culturedcode.com/things/)：复选框是主角，列表没有卡片、只有留白和细线。借鉴：复选框做大（20px）、行没有底色。

## 3. 字体（许可都允许随应用打包）
| 用途 | 字体 | 许可 | 备注 |
|---|---|---|---|
| 中文界面（三主题共用） | **MiSans** | [小米字体许可](https://hyperos.mi.com/font/faq)：免费商用、可嵌入软件，须在软件内注明，**不得修改（含子集化需谨慎）**、不得单独分发 | 字面大、x 高度高、数字清楚；备选 [HarmonyOS Sans SC](https://github.com/huawei-fonts/HarmonyOS-Sans)（条款几乎相同） |
| 中文兜底 / 若要子集化 | Noto Sans SC（思源黑体） | [OFL 1.1](https://github.com/notofonts/noto-cjk) | 可子集化，包体最可控 |
| C 主题中文标题 | **霞鹜文楷 LXGW WenKai** | [OFL 1.1](https://github.com/lxgw/LxgwWenKai) | 手写感，只用在标题和标签，不用在正文 |
| 西文与小数字（共用） | **Geist / Geist Mono** | [OFL 1.1](https://github.com/vercel/geist-font) | 路径用 Mono，数字开 tabular-nums |
| A 大数字 | **Instrument Serif** | [OFL](https://fonts.google.com/specimen/Instrument+Serif) | 窄衬线，大字号下有杂志感 |
| B 大数字 | Geist 200（极细） | OFL | 暗底上细字更亮、更"仪表" |
| C 大数字 | **Bricolage Grotesque** 800 / 宽度 75 | [OFL](https://fonts.google.com/specimen/Bricolage+Grotesque) | 窄粗、有墨水陷阱，卡通但不幼稚 |

没选：Inter / Space Grotesk（AI 稿最常见）、DM Sans / Manrope（中西混排时比 MiSans 显小）。

**字号阶梯**（中文和西文分开：MiSans 字面约占 0.9 em，Geist x 高度约 0.53 em，同 px 下西文显小，所以数字比同行中文大 1–2px）
- 中文：12 辅助 / 13 次要 / 15 正文与行名 / 18 分组标题 / 24 页标题（C 用文楷 28）。中文最小 12px，不再用 11。
- 西文数字：13 路径（Mono 12）/ 16 行内大小 / 28 容量柱标注 / 56 C 盘读数 / 220–300 主数字。
- 行高：中文界面 1.4，说明文字 1.65；大数字 0.85。

## 4. 开源设计系统里用到的原则
- [Radix Colors 12 阶](https://www.radix-ui.com/colors/docs/palette-composition/understanding-the-scale)：1–2 背景、3–5 组件底色、6–8 边线、9 纯色、11–12 文字。三主题色板都按这个角色分层（bg / surface / line / ink1-3）。
- [shadcn/ui](https://ui.shadcn.com) 与 Radix Primitives：主题只换 CSS 变量、结构不动——这就是"三主题共用一套布局"的实现方式。
- [Vercel Web Interface Guidelines](https://github.com/vercel-labs/web-interface-guidelines)：比较用 tabular-nums、焦点可见、破坏性操作要确认、按钮文案写清结果（"清理 32.1 GB"而不是"确定"）。
