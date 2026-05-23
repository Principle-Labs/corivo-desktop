# Corivo Roadmap: Validate → Reflect → Automate

> 内部战略路线图 · v1
> 这份文档不是对外 vision，是 founder 用来做产品决策的 north star。
> 它有立场、有可证伪的赌注、有明确的失败/成功条件。
> 跟 [vision-public.md](vision-public.md) 互补：那一份是"我们告诉世界什么"，这一份是"我们相信什么是真的"。

---

## TL;DR

Corivo 的护城河押在三件事上，**严格按顺序**：

1. **Project 结构能被 AI 准确推断出来**（而不是用户手建）
2. **基于这个结构能产生用户主动想看的洞察**
3. **基于洞察建立的信任，agentic 才能立得住**

任何一阶失败，下一阶都没意义。所以这份路线图是**严格分阶段**的，每一阶都有明确的验证目标和退出条件。**Phase N 没通过之前，不写 Phase N+1 的代码。** 这是契约。

---

## 现实约束（先读这一节）

这份路线图执行有一个硬外部时间窗口：

**团队给的时间是 1-2 周（争取到 2 周）。这是 Phase 1 的窗口，不是全部三阶段的窗口。**

意思是：

- Phase 1 不只是内部验证，还要在 ≤ 2 周内**产出一份可向团队展示的结论**
- Phase 2/3 的实际开始时间，取决于 Phase 1 后的团队 go/no-go 决议
- 如果 Phase 1 在 2 周内拿不到 ≥ 70% 准确率，就做出诚实的"暂停"决定，不要自我说服再延一周

把这个写在最前面，是为了让所有后续选择对得起这个约束。

---

## Phase 1 · 验证（Validate）

### 要验证的赌注

> "AI 创建项目 + 用户重命名/调整"这种归属机制，能在不让用户感到痛苦的前提下，把日常工作准确地组织成 Project → Task → Event 三层结构。

这是 Corivo 整个产品的物理基础。如果这一步不成立，对外 vision 里的 5 阶段全部是空中楼阁。

### 不需要构建的东西

Phase 1 不写新功能。需要测试的全部基础设施都在了：

- `/now` 三栏视图（project / task / event timeline）
- `event_attributor`（identity-anchor matching）
- `retrospective_reviewer`（reassign + split）
- `task_narrator` + `project_rollup`
- `event_boundary` / `event_task_attribute` 等 prompts

这一阶段做的是**测量**和**调参**，不是构建。

### 唯一的事

**Founder 全职用 Corivo 一周。每天结束时审视 `/now`，回答一个问题：**

> "这张图，是不是我今天工作的真实写照？"

### 执行节奏（受 2 周外部窗口约束）

**Week 1 · 基线测量**
- 用现有 build 全职 dogfood
- 每天 5 分钟简短记录（见下方"每日记录"）
- 不调 prompt、不改 anchor 阈值，只测量

**Week 1 结束 · 中期评审**
- 看 7 天的归属准确率、手动移动次数、project 数量
- 决定 Week 2 调什么：prompt? anchor 阈值? retrospective_reviewer 策略?
- **最多调一次**，不要把 Week 2 也变成"测试 + 调参 + 测试"的三明治

**Week 2 · 调参后的二次测量**
- 应用 Week 1 决定的调整
- 再 dogfood 5-7 天
- 这次的数据就是给团队看的最终答案

**Week 2 结束 · 向团队呈现**
- 不达标 → 诚实暂停（不要说服自己再来一周）
- 达标 → 进入 Phase 2A（founder 阶段）

### 退出条件（量化）

满足以下全部，才能进入 Phase 2：

| 指标 | 目标 |
|------|------|
| 归属准确率（事件落到正确 project，不需手动移动） | ≥ 70% |
| 每日手动移动次数 | < 5 |
| AI 创建的"无意义项目"（一眼看不出在说什么） | 0 个/周 |
| Founder 主观判断"这是我真实的一天" | 7 天里 ≥ 5 天 yes |

70% 这个数字是发布门槛的近似值。低于此，Phase 2 引入 alpha 用户后会立刻流失。

### 失败信号

任意一个出现，就要回头改：

- **Project soup**：一周 AI 创建 > 15 个 project → 颗粒度太碎
- **Project monolith**：不相关工作被塞进一个 project → 颗粒度太粗
- **Anchor poverty**：新建 project 拿不到足够的 anchor，永远无法自归属
- **Drift 没被接住**：已经偏离原意的 task，retrospective_reviewer 没发现/拆分
- **死事件**：长期"未分类"的事件越积越多

### 每日记录

要在 Phase 1 结束时回答 exit criteria，必须每天 5 分钟简短记录：

- AI 建了几个 project？哪些合理？哪些奇怪？
- 我手动移了几次？为什么需要移？
- /now 显示的"今天" 和我大脑里的"今天" 差距在哪？
- 哪些 prompt 输出最让我困惑？

### Phase 1 输出

- **决策**：Project 结构是否成立？
  - **成立** → 进入 Phase 2
  - **部分成立** → 调 prompt / anchor 阈值，再来一周
  - **不成立** → 暂停 Phase 2/3，重新思考核心结构

---

## Phase 2 · 洞察（Reflect）

> ⚠️ Phase 1 通过之前，**不开始 Phase 2 的工作**。

### 要验证的赌注

> "一旦归属可信，Corivo 就能产生用户**主动想看**的洞察 —— 不是 dashboards 那种泛泛统计，而是'我自己看不见但 Corivo 看得见'的真东西。"

### 已有基础

- `/coach` 页面（Stage 9b 刚上线 goal-aware coach + drift evaluator）
- `task_narrator` / `project_rollup`
- 完整的 event/task/project 数据 + facts 表

也就是说，Phase 2 的"基础设施"已经在了。要做的是**对洞察类型的产品验证**和 UX 打磨，不是从零搭。

### 候选洞察类型（待筛选）

| # | 类型 | 例子 |
|---|------|------|
| 1 | 时间真相 | "X 项目本周累计 12h，commits 只增加 1" |
| 2 | 漂移检测 | "你今天目标是 X，已 2h 在 Y" |
| 3 | 重复模式 | "你第 4 次切回这个 bug" |
| 4 | 跨项目关联 | "X 和 Y 在解决同一个问题" |
| 5 | 沉睡项目 | "你 3 周没碰过 Z" |
| 6 | 节奏检测 | "你今天上午高产，下午掉 60%" |
| 7 | 卡点识别 | "你在 Z 上停滞 2 天，没有可见进展" |

候选 7 个，目标是筛出 ≥ 3 个 founder 真心觉得"自己想不到"的。其余删掉。

### 用户范围 progression（A → B → C，每一步都是 gate）

| 阶段 | 用户 | 进入条件 | 时长（粗估） |
|------|------|---------|----------|
| **2A** | Founder 自己 | Phase 1 通过 + 团队 go | 1-2 周 |
| **2B** | 1-3 个熟悉的朋友 | 2A 满足"founder 觉得这件事真的能做" | 2-3 周 |
| **2C** | 5-10 个外部 alpha | 2B 满足朋友也认可 | 3-4 周 |

**关键原则：每一步都可以独立判定"值不值得继续"。** 2A 不通过不进 2B，2B 不通过不进 2C。这跟 Phase 1 → Phase 2 的 gate 是同一个逻辑。

最大风险：2A 已通过，2B 拿到混合反馈（朋友友善但暧昧）。要做好"朋友说还行 ≠ 真正认可"的辨别——具体方法：看朋友是不是**主动**截图分享给别人，而不是问他"你觉得怎么样"。

### 退出条件

| 指标 | 目标 |
|------|------|
| Founder 自己认可"真正有价值"的洞察类型 | ≥ 3 个 |
| 洞察出现频率 | 既不噪音也不无感 |
| 每条洞察的 founder 反应 | "这个我自己想不到" 或 "这个值得我停下来看一下" |
| 外部 alpha 用户认可的洞察类型（若引入） | ≥ 2 个 |

### 失败信号

- 洞察都是"app 的统计"——把 RescueTime 翻译了一遍而已
- 洞察让人想关掉提醒——push 做了但价值不够
- 洞察看完不知道该怎么办，但又不到能 take action 的程度（Phase 2 不是 Phase 3，洞察不需要 actionable，但要 truthful + useful）

### Phase 2 输出

- **沉淀的"洞察清单"**：哪些留、哪些删、哪些改 UX
- **决策**：能不能进入 Phase 3
  - **可以** → Phase 3 的具体范围由 Phase 2 揭示出的"最痛缺口"决定
  - **不可以** → 重新打磨洞察层，不进 agent

---

## Phase 3 · 执行（Automate / Act）

> ⚠️ Phase 2 不通过，Phase 3 不存在。

### 要验证的赌注

> "在前两阶建立的信任之上，Corivo 可以**主动起草、主动行动、主动出现** —— 而不被用户视为骚扰或越权。"

### 这一阶段现在不细化

故意的。原因：

1. Phase 3 的具体形态，应该由 Phase 2 揭示出的"最痛缺口"决定，不是现在拍脑袋
2. 任何在 Phase 1/2 完成前对 Phase 3 的细化，都会扭曲前两阶段的判断

### 第一个 milestone（founder 定义的"Phase 3 真正开始"那一刻）

> "别人给我发了一条消息，Corivo 主动说'我可以帮你加到日程上'。"

这个例子的关键不是"加日程"这个动作本身，而是要走通这条链路：

1. **Corivo 看到了消息**（已有：observation 流）
2. **Corivo 理解消息和某个 project 的关联**（已有：identity-anchor matching）
3. **Corivo 识别出"这是一件可以替你做的事"**（**待建：action recognition**）
4. **Corivo 主动开口请求授权**（**待建：proposal UI**）
5. **Corivo 执行 + 汇报结果**（**待建：calendar integration**）

走通这一步，后面 Stage 4 起草、Stage 5 代办都是同构的——只是接入更多 action backend。所以这是 Phase 3 的最小可见形态，也是 MVP。

### Phase 3 的演进方向（仅供参考，不是承诺）

- **3a 被动建议**：识别可 take action 的瞬间，列出来等用户点击
- **3b 主动提议**："我注意到 X，要不要我帮你 Y？"（这就是上面那个 milestone）
- **3c 小步自动**：例行 + 低风险的事，征得一次性授权后自动做（如自动添加日程）
- **3d 更大代办**：草稿、汇报、邮件——等 3c 信任建立后才考虑

### Phase 3 进入门槛

不仅是 Phase 2 退出条件全部满足，还包括：

- ≥ 5 个外部 alpha 用户连续使用 ≥ 4 周，留存率 ≥ 60%
- 至少有一类洞察被外部用户**主动转发或截图分享**

如果这两个没达成，说明信任还没真正建立，agent 不该上。

---

## 横向风险

### 隐私文案 vs 实际架构

**已选定方案：技术上转向本地优先 + 诚实披露剩余的 cloud 调用。**

具体计划：
- **截图获取**：转向 macOS A11Y + OCR，不再发原始截图给 vision LLM
- **总结 / 事件归属 / 边界判断**：仍然调用 cloud LLM（Gemini / Codex），但传过去的是 OCR 抽取出的**文本**，不是原图
- **披露三处**：README、设置页、首次启动 onboarding，明确说明"截图本地处理，文本摘要走 cloud LLM"

实质上是个更温和、更诚实的"all on your computer"——没有图像离开本机，但文本仍然会。措辞上不能再说 all。建议参考措辞：

> "Screenshots stay on your machine. Text summaries are processed by cloud LLMs (Gemini / Codex)."

**deadline**：在最终方案确定后定稿对外措辞，并同步修改 [vision-public.md](vision-public.md)。

### Schema 与无向后兼容

Project 结构、anchor schema、事件颗粒度——在 Phase 1/2 会反复改。继续按 [CLAUDE.md](../CLAUDE.md) 里"少考虑向后兼容"的做法，purge-and-apply 不要犹豫。

### Founder 自身的耐心

最大的产品风险：Phase 1 没通过就跳 Phase 2，Phase 2 没通过就开始 Phase 3。每一阶往前的诱惑都很强，因为前面"看着像在工作了"。

**写明这一点是为了让自己有契约去抵抗。**

---

## 时间预算

| 阶段 | 预算 | 进入条件 |
|------|------|---------|
| **Phase 1 · 验证** | **2 周（含 1 次迭代）** | 已开始 |
| **Phase 2A · 洞察 (founder)** | 1-2 周 | Phase 1 退出条件全部满足 + 团队 go |
| **Phase 2B · 洞察 (close friends)** | 2-3 周 | 2A founder 主观认可 |
| **Phase 2C · 洞察 (public alpha)** | 3-4 周 | 2B 朋友也认可 |
| **Phase 3 · 执行** | 8-12 周（粗估） | Phase 2C 退出条件 + 留存 ≥ 60% |

总跨度：约 4-6 个月走完三阶。但**每一阶都是独立 gate**，任何一阶不通过都正当地暂停。

---

## 已决议事项（v1）

1. **Phase 1 时长**：2 周（迭代上限 1 次），受外部团队 deadline 约束
2. **Phase 2 用户范围**：A → B → C 顺序进入，每一步都是 gate；C 仅在 A+B 都认可后才打开
3. **外部 deadline**：团队给 1-2 周作为 Phase 1 窗口，结束后呈现结果争取后续时间
4. **Phase 3 第一个 milestone**：Corivo 看到消息后主动提议加日程（走通 5 步链路）
5. **隐私方案**：A11Y + OCR 替代 vision LLM 处理截图；文本摘要仍走 cloud LLM；如实披露

## 仍未决（v2 再回来）

- Phase 2 各 gate 的"通过"是否需要更量化的指标，还是 founder + 朋友的主观判断够了
- Phase 3 第一个 milestone 之后的排序（草稿 vs 周报 vs 邮件 vs 通知）
- Phase 1 的"团队呈现"用什么形态：slide / live demo / metrics dashboard / 三者混合
