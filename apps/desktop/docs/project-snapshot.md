# Corivo · 项目快照与思路

> 截至 2026-04-26 · 一份内部用的状态总览
>
> 用途：休息几天回来快速重新进入；新人加入时一份钟读完了解项目；自己思路不清时拿来梳理。
>
> 配套文档：[design-decisions.md](design-decisions.md)（决策主干）、[vision-public.md](vision-public.md)（对外愿景）。

---

## 一、起点：要解决的真问题

知识工作者的真实工作状态：

- 同一天在 5–10 个项目 / 任务之间切换，没有清晰边界
- 上下文散落在 IDE、浏览器、Slack、Notion、终端
- 个人答不出"我今天究竟在哪几件事上花了多少时间、推进了什么、卡在哪"
- 通用 productivity tracker 要求自己上报，本身就是新负担

Corivo 的初始下注：**屏幕已经知道一切。**只要持续观察 + LLM 蒸馏，就能 surface 用户自己看不见的东西，做出比"用户告诉 AI"更深的工具。

---

## 二、三次架构尝试，三次被同一前提证伪

### 第一次：GUM / proposition / push 推送

- **想法**：把用户建模成一组 propositions（PROPOSE → SIMILAR → REVISE），高置信度的通过 Dynamic-Island 风格的 overlay 主动推送
- **它给了什么**：清晰的累积模型 + focus_session 聚合 + signal 锚点等基础设施
- **为什么没成**：proposition 描述的是"用户"，不是"用户在做的工作"；push 是错的形态，"高置信度" ≠ "现在该说话"；project_resolver 用语义命名导致 25 条 work_context 全塞进同一个名字相近的 project
- **教训**：模型可以非常准确地了解你，仍然说不出任何有用的话

### 第二次：brain 重构（fact / thread / narrator）

- **想法**：拆掉画像层，换成结构化 fact 抽取 + narrator 服务
- **它给了什么**：干净的 `facts (kind/subject/value)` 抽取范式，narrator 产出，DbInstant 时间层
- **为什么没成**：解了 V1 的"画像无用"，但代价是把 Project 层 + signal 锚点 + focus_session 聚合一起删了。`/facts` 页退化成平铺原子，看不到"在哪个项目推进了什么"
- **教训**：保留好的（fact / narrator / time），但项目层是必须的

### 第三次：event / task / project（当前主干）

- **想法**：回到 V0 的三层结构（projects → tasks → events），保留 brain 的 facts + narrator，加身份锚点 + 周期性 retrospective reviewer
- **它给了什么**：三层结构匹配真实工作；身份锚点让归属稳定；/now 三栏 UI；自洽的待分类回收
- **为什么仍然不满意**：dogfood 后用户说

  > "用项目视角回顾工作进度体验最好。**没有错得离谱的，但记录的都是我做过的事，所以用处不大。**"

  准确性 OK，问题不是 prompt、不是结构，而是 **被动记录用户已知的事，本身没有用户价值。**

### 三次共享的错误前提

> "只要 context 做够，建议自然出来。"

三次被证伪。Rewind / Granola / 各种"自动工作日志"也都死在这一关。

---

## 三、转折认识

**新的判断**：好建议的形状决定了 context 应该是什么形状，不是反过来。

- 没有锚（用户在追什么）→ 系统只能复述观察
- 有了锚 → 同一个观察可以变成"你今天第 3 次回到这个 bug"或"X 项目投入 12 小时无进展"

Corivo 因此重新定位：

> 从 "自动总结你工作的 AI" → 到 **"目标对照式行为教练 → 最终演化为 AI 同事"**

---

## 四、当下状态（2026-04-26）

### 刚 ship 的：Stage 9b · goal-aware coach v1

- 用户随时在 /coach 顶部一句话录入目标（"本周末交 demo"），无 deadline 字段——时间信息走自然语言
- 每次产生新 event → `goal_evaluator` 跑一次 LLM 调用 → 每个 active goal 输出 `on_track / drifting / off_track` + 一句话理由
- /coach 页顶部 = 目标输入 + 列表；中段 = drift 评估 feed（绿 / 黄 / 红 三色徽章）；底部保留原 scout/coach/friend 三卡片

### 信任阶梯当前位置

```
Observer 观察者     ← 接近完成（capture / observation / event 管道稳了）
   ↓
Mirror   镜子       ← Stage 9b 落了 v1 的子集（drift 检测）
   ↓
Coach    教练       ← Stage 9b 落了 v1 的子集（目标对照）
   ↓
Assistant 助手      ← 没碰
   ↓
Delegate  代理      ← 没碰
```

### 栈状态

- `cargo check` / `pnpm build` / 6 个新单测全绿
- Schema 版本 104（goals + goal_event_evaluations + Stage 10 项目合并相关）

---

## 五、核心要解决的问题

### 一句话

> **Corivo 必须告诉用户一件用户不知道、但系统能告诉他、并且用户会真的高兴的事。**

这是 Corivo 唯一要赢的那一仗。其他所有架构问题（要不要塌缩 project 层、push 何时回来、要不要做 substrate）都是这一仗附带的。

### 拆开看

- **信号源**：必须从用户**看不见的地方**来 —— 重复回访、停滞、跨上下文连接、目标偏离
- **表达**：必须**具体、有名词** ——「你今天工作认真」是废话，「X 项目 12 小时 0 进展」是有用的
- **时机**：必须**克制** —— push 在 wedge 验证前不要做，先让用户主动打开 /coach
- **评估**：必须**可验证** —— 用户用一周后会不会每天打开 /coach？这是唯一标尺

### 为什么这一仗赢得起 / 输不起

赢了 → 后面所有事（v1.5 路径教练、v2 改进教练、Year 2 Assistant、Year 3 Delegate、甚至更野的 personal AI substrate）都有意义。

输了 → 重做 agent / 重做 push / 找新用户都救不了。问题不在执行，在于赌错了 wedge。

---

## 六、接下来的可能路径

按时间轴 + 信心分级。

### 短期（接下来 1–2 周）

**必做**：把 Stage 9b 真用一周。

- 自己用 Corivo 准备 Corivo 的 demo，每天写真实目标
- 记录哪些 drift 判断有用 / 哪些尬 / 哪些错
- 一周后还会不会主动打开 /coach
- 这一步的输出 ≠ "代码改进"，而是 **"goal × event 这个机制本身值不值得 double down"**

**条件性做**：

- 输出有用 → 进入 v1.5（路径教练：怎么帮你达成目标）
- 输出准但 reason 空 → 调 prompt
- 输出不被打开 → 退一步，重新想 wedge

### 中期（Year 1，2 人 + 无限 Claude）

**Year 1 唯一目标**：把 Mirror + Coach 做到 OS 大厂做不出的深度。

候选 Mirror 模式（按已识别的潜在价值排序）：

| 类型 | 例子 | 状态 |
|---|---|---|
| 目标对照 | "你说要交 demo，但这 30 分钟在写别的项目" | ✅ 已 ship v1 |
| 重复回访 | "你今天第 N 次回到这个 bug，累计 N 小时" | 待做 |
| 停滞检测 | "X 项目投入 N 小时但无进展信号" | 待做 |
| 隐性消耗 | "你刷 Twitter 累计 N 分钟" | 待做 |
| 盲点提醒 | "PR 还没 merge / doc 没动" | 待做（需外部数据源） |

**明确不做**：

- ❌ 通用 agent 平台（赌不过 Manus / Devin / Operator）
- ❌ 生产力 dashboard（不卷可视化）
- ❌ 跨上下文召回（Year 2 再考虑）
- ❌ AI 个性 / 情绪表达（路径风险大）
- ❌ 截图管线重做（事件触发 / 本地 OCR / 双管道 / 视频上传都暂缓 —— wedge 验证后再优化基础设施）
- ❌ Push / overlay 继续暂停

### 长期（Year 2–3，两个有意思的分叉）

**分叉 A · Assistant + Delegate（产品深耕路线）**

- 替你起草下一步（draft / 清单 / 跟进项 / 邮件回复）
- 替你跑重复流（agent 自动执行 + 报告）
- 对手变成 Apple Intelligence / Microsoft Copilot 这类 OS-级整合 —— 窗口期估计 1–2 年

**分叉 B · Personal AI Substrate（基础设施野心）**

- Corivo 不止是产品，是你**所有 AI 工具的 context 提供方**
- ChatGPT / Claude / Cursor / Notion AI 想知道你是谁、在做什么、卡在哪 —— 都从 Corivo 读
- "同事能力"是 first-party application，substrate 才是真正的护城河
- 这条路更野、也更不确定，但天花板高一档

A 和 B 不互斥，但精力优先级要选。Year 2 初看 v1.5 / v2 的留存数据再决定。

---

## 七、最大的不确定性（承认下来）

1. **dogfood 自己一周不一定有结论** —— 一个用户的样本太小。可能需要找 5–10 个早期用户。
2. **个人 AI 这个赛道竞争非常拥挤** —— Manus / Rewind / Personal.ai / OS 大厂都在做。差异化是"它知道你"，但这张牌打不打得出来取决于 Mirror + Coach 的实际深度。
3. **2 人团队 + Year 1 的天花板** —— Mirror + Coach 高质量已经接近极限。如果 Stage 9b 的 wedge 不立，加人也救不了。
4. **大厂下场时机** —— Apple Intelligence / Microsoft Copilot 越来越下沉到 OS 级。Corivo 能跑多远取决于它们什么时候真正动手。1–2 年的窗口期是核心赌注。

---

## 八、附：关键文档索引

- [design-decisions.md](design-decisions.md) — 决策主干（journey + commitment + spec + open questions）
- [vision-public.md](vision-public.md) — 对外用愿景文档（官网底稿）
- [event-task-project-spec.md](event-task-project-spec.md) — V3 架构 spec（已落代码，作为实现细节参考）
- [../CLAUDE.md](../CLAUDE.md) — 给 AI 协作者的项目说明
