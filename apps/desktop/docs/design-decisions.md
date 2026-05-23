# Corivo: 设计与决策

> 状态：v1（决策主干，取代 `event-task-project-spec.md` 作为顶层文档；原 spec 降为实现细节参考）
> 用途：内部决策记录 + 向前的产品 spec
> 配套：对外/官网用的中后期愿景版另立文档（`vision-public.md`，待写）
>
> 核心判断：
> - V0 / brain / event-task-project 三次架构尝试都假设"context 做够，建议自然出来"，三次被证伪
> - 新判断：**好建议的形状决定了 context 应该是什么形状**，不是反过来
> - Corivo 重定位：从"自动工作日志"→"目标对照式行为教练 → AI 同事"
> - 差异化只有一条：**它知道你**。这一点立住，所有功能成立；立不住，跟通用 agent 没区别

---

## Part 1 · The Journey

### 1.1 起点：要解决什么

知识工作者的真实工作状态：

- 同一天在 5–10 个项目 / 任务之间切换，没有清晰边界
- 上下文散落在 IDE、浏览器 tab、Slack、Notion、终端
- 个人答不出"我今天究竟在哪几件事上花了时间、推进了什么、卡在哪"
- 通用 productivity tracker 要求自己上报，本身就是新负担

Corivo 的初始下注：**屏幕已经知道一切。**只要持续观察，就能 surface 用户自己看不见的东西，做出比"用户告诉 AI"更深的工具。

### 1.2 第一次尝试：GUM / proposition / push（截至 `334a5e8`）

**架构**

```
screenshots → vision LLM → propositions (PROPOSE / SIMILAR / REVISE)
            → work_contexts → project_streams → projects
                                                ↓
                            high-confidence ones → push overlay (Dynamic Island 风)
```

参考学术 GUM（General User Model）—— 把用户建模为一组 proposition，随观察 propose / similar / revise。

**它给了什么**

- 一套清晰的累积模型
- `focus_session` 聚合、`reconcile` 审计、`work_context_signals` 锚点等基础设施
- 第一次跑通 screenshot → LLM → 持久化 → UI 的全管道

**为什么没成**

- **propositions 描述的是"用户"，不是"用户在做的工作"** —— "用户偏好深色主题"对"今天怎么干完活"几乎没用
- **push 是错的形态** —— "高置信度" ≠ "现在该说话"，多数 push 像打扰而不是帮忙
- **project_resolver 用语义命名** —— 25 条 work_context 全被塞进名字相近的 project，没有身份锚点
- 第一次证伪 "context 做够，建议自然出来" 的假设：**模型可以非常准确地了解你，仍然说不出任何有用的话**

### 1.3 第二次尝试：brain 重构（`7e45a66`）

**架构**

整块拆掉 V0，换成 `facts / threads / thread_facts / links / push_events` + `narrator`。

**它给了什么**

- `facts (kind / subject / attribute / value / unit / confidence)` —— 干净的结构化抽取
- `narrator` 服务（产出 summary / resolution / open_questions_advice）
- `DbInstant` + 全局 `Clock` 时间层

**为什么没成**

- **删掉了 Project 层** —— 但用户要的就是"在哪个项目推进了什么"
- **删掉了 signal 锚点** —— 没有锚 → 没法稳定归属
- **删掉了 focus_session 聚合** —— event 这个粒度没了
- `/facts` 页面退化成 `[intent] recall detector query ... = 修复 ...` 平铺原子，看不见工作单元

新范式解了 V0 的"画像无用"，代价是把项目级别的工作进度一起删了。`facts / narrator / time` 保留，其余退回。

### 1.4 第三次尝试：event / task / project（当前分支，`e28d434` → `f935d2a`）

**架构**

```
capture_loop → observation_ingest → observation_session
                                          ↓ EventEmitted
                                    event_attributor (anchor + LLM)
                                          ↓
                            projects / tasks / events / facts
                                          ↑
                  task_narrator / retrospective_reviewer (周期性)
```

回到 V0 的三层（projects / tasks / events），保留 brain 的 facts 抽取与 narrator 思路，加 `project_identity_anchors` 身份锚点。Stage-1 → Stage-8 共 ~15 commits 把这套从 V0 改建出来。

**它给了什么**

- 三层结构匹配真实工作（项目 / 任务 / 事件）
- 身份锚点让 project 归属稳定（不再靠语义命名碰运气）
- `retrospective_reviewer` 处理待分类 / 漂移任务，自洽
- /now 三栏 UI 把架构展示给用户

**为什么仍然不满意**

dogfood 后的真实反馈（2026-04 中下旬）：

> "用项目视角回顾工作进度体验最好。**没有错得离谱的，但记录的都是我做过的事，所以用处不大。**"

这句话是这次重构最重要的输出。它说明：

- 准确性已经 OK，问题不是 prompt、不是结构
- 问题是 **被动记录用户已知的事，本身没有用户价值**
- 这是 productivity 工具的经典死亡陷阱（Rewind / Granola 类工具都在这一关挣扎）
- 用户对自己的过去不那么好奇 —— 真正有价值的是 **用户不知道、但系统能告诉他的事**

### 1.5 转折：从"自动日志"到"目标对照式行为教练"

三次尝试共享同一个错误前提："只要 context 收得够好，建议自然出来。"三次被证伪。

新的判断：**好建议的形状决定了 context 应该是什么形状**，不是反过来。没有锚（用户在追什么），系统只能复述观察；有了锚，相同的观察可以变成"你今天第 3 次回到这个 bug"或"X 项目投入 12 小时无进展"。

Corivo 因此从 "自动总结你工作的 AI" 重新定位为：

> **目标对照式行为教练 → 最终演化为 AI 同事**

---

## Part 2 · Current Commitment

### 2.1 一句话愿景

> **Corivo 是住在你电脑里的 AI 同事 —— 看你工作，理解你的方式，再替你工作。**

跟主流 AI agent 产品的结构性差别：

| | 接受指令的 AI（Manus / Devin / Operator） | Corivo |
|---|---|---|
| 起点 | 用户下指令 | 系统先看用户 |
| 上下文 | 单次任务的 prompt | 持续累积的工作行为 |
| 关系 | 工具 ↔ 使用者 | 同事 ↔ 同事 |
| 信任 | 一次完成或失败 | 逐级解锁 |

唯一的差异化点：**它知道你**。这一点立得住，下面所有事都能做；立不住，跟其他 agent 没区别。

### 2.2 信任阶梯（5 级）

每一级独立可用、可 demo、可销售。每一级要用户感觉到价值才能解锁下一级 —— **earn the right to act**。

| 级 | 名称 | 用户感受 | 系统能力要求 |
|---|---|---|---|
| 1 | **Observer 观察者** | "它在看着我工作" | 稳定捕获截图 → observation → event |
| 2 | **Mirror 镜子** | "它告诉了我自己看不见的事" | 跨 event 模式识别（投入 / 卡顿 / 重复） |
| 3 | **Coach 教练** | "它帮我守住今天要做的事" | 用户声明目标 × 实际行为对照 |
| 4 | **Assistant 助手** | "它帮我准备好了下一步" | 根据上下文起草 / 列清单 / 备工件 |
| 5 | **Delegate 代理** | "它替我把那件事做完了" | 固化的重复流程自动执行 + 报告 |

**当前位置：Observer 接近完成。Year 1 的目标是 Mirror + Coach 做到极致。**

### 2.3 差异化：为什么是"知道你的 AI"

四个对手类别，Corivo 的相对位置：

| 对手 | 它们做什么 | 它们没做什么（Corivo 的入口） |
|---|---|---|
| **通用 agent**（Manus / Devin / Operator） | 接 prompt 跑任务 | 不持续观察用户、不知道用户是谁 |
| **个人记忆**（Rewind / Personal.ai） | 录屏 / 录数据 + 检索 | 不主动 —— 要用户问才有价值 |
| **OS 内 AI**（Apple Intelligence / Copilot） | 跨 app 浅整合 | 浅 —— 不深入工作语境 |
| **生产力工具**（Notion AI / Granola） | 文档 / 会议 / 自动总结 | 单点 —— 不跨上下文 |

Corivo 的赌注：**深度 vs 广度。**OS 大厂能做广，但做不到对一个用户的深度理解（10 亿用户的隐私 / 一致性约束注定他们只能做浅层）。我们能。

时间窗口：估计 1–2 年，OS 大厂下场之前必须把根扎到他们挖不动的深度。

### 2.4 Year 1 范围：Mirror + Coach 做到极致

**Mirror（镜子）要做的事**

- "你今天第 N 次回到 X"（重复回访检测）
- "X 项目投入 N 小时但没有进展信号"（停滞检测）
- "你刷推 / 上 YouTube 累计 N 分钟"（隐性时间消耗）
- "你提的 PR 还没 merge / 上次说要写的 doc 还没动"（盲点提醒；跨外部数据源时再做）

**Coach（教练）要做的事**

- 用户随时一句话录入目标 / 意图（"本周末交 demo"），可改可删（UX 方案 = **B**）
- 系统持续把当前 event / 行为分布对照声明的目标，输出 on-track / off-track / drifting
- 主动 surface "你这 30 分钟在做的事跟你说要做的不是同一件"

**不做的事**（明确划线，防止漂移）

- ❌ 不做通用 agent 平台（不接 browser action、不做通用 RPA）
- ❌ 不做生产力 dashboard（不卷可视化）
- ❌ 不投入跨上下文召回（Year 2 再考虑）
- ❌ 不投入 "AI 个性 / 情绪表达"（路径风险大，Year 2 决策）
- ❌ 不重做截图管线（事件触发 / 本地 OCR / 双管道 / 视频上传都暂缓 —— wedge 验证后再优化基础设施）
- ❌ Push / overlay 继续暂停，wedge 阶段不做主动通知

### 2.5 资源与节奏

- 当前团队：2 人 + 无限 Claude
- Year 1 末扩展上限：4 人
- **Year 1 唯一目标**：Mirror + Coach 做到 OS 大厂做不出的深度，让 100–1000 个真实用户每天打开
- **Assistant（Year 2）入场条件**：Mirror + Coach 至少一项被用户主动每天使用
- **Delegate（Year 2 末或 Year 3）入场条件**：Assistant 用户愿意把"系统准备好的下一步"交给系统直接跑

---

## Part 3 · 此刻 · 下一步动作

> 写于 2026-04-26。Part 1 / 2 已 close，Part 4 forward spec 此刻仍 draft 不出来 —— 不是因为信息不够，是因为不该 draft。本节记录这个卡点与暂定动作。

### 3.1 此刻的卡点

写完 Part 1 / 2 之后，再次面对"下一步该写什么代码"，三条路都看得见，每条都像新的承诺：

- **继续建**（按 Mirror + Coach 推 Stage 9-14）—— 风险：重演前三次"build first → 没人觉得有用"
- **停下来验证** —— 感觉像在拖延
- **推倒重来** —— 不舍得 Stage 1-8

三条都对，三条都不舒服。

**判断：这不是信息不够的问题，是缺外部反馈把问题收窄。** 内部再思考一轮得不到答案 —— 前三次每次都做了"再思考一轮"。

### 3.2 这一周的唯一动作

**找 3 个愿意当 alpha 用户的人，在写一行新代码之前。**

每人需明确同意：1-2 周后拿到粗糙版本 → 每天用 → 每周给一次反馈 → 至少 4 周。

候选人门槛：

- 非程序员，或对粗糙产品宽容的程序员
- 信任度足够会说真话（不客气敷衍）
- 自己有"今天到底干了啥 / 是不是又漂走了"的困惑

**不找：** 投资人、做 AI 产品的同行、会"评价产品定位"的人。找会用产品的人。

### 3.3 为什么这是 prerequisite，不是 nice-to-have

- 不动代码 → 没法逃回工程舒适区
- 找不到 3 个 yes → 愿景层（`vision-public.md`）有问题，应停止投人力先回去重想
- 找到 3 个 yes → 1-2 周后必须 ship → "该 ship 什么" 会自动从无限选项收窄

**Part 4 forward spec 在 4 周后才有可能 draft 得对** —— 那时是 3 个真实用户用 4 周后的反馈倒推出来，不是脑里推出来。前三次的失败模式就是脑里推 spec、再 build、再发现 spec 错了。

### 3.4 这周的禁止事项

写下来防止自己滑回去：

- ❌ 不写 Part 4 / Part 5 内容
- ❌ 不重排阶段规划
- ❌ 不研究 Yansu / Vida 的实现细节
- ❌ 不决定 project 是否塌缩成 goal
- ❌ 不决定截图本地化方案
- ❌ 不打开 IDE 写新代码

这些问题都真，**但它们的答案会在 3 个真实用户用 4 周之后自己浮出来。** 在那之前回答都是猜的，跟前三次一样。

### 3.5 退出条件

- **3 个 yes 拿到** → 进入"1-2 周内 ship 给他们的最小版本"工作模式，反推那周必须做什么；这时才开始 draft Part 4
- **8 个发出去回收不到 3 个 yes** → 不是"再发更多"，而是回到 `vision-public.md` 重想愿景层

---

## Part 4 · Forward Spec（Mirror + Coach）

> 已锁定：**goal 输入 UX = 方案 B（任意时刻一句话录入，可改可删）**。
> 4.1 已起草架构决策日志（见下）；其余子节（具体 UX / first ship-able 版本）在 Part 3 退出条件满足之后再起草。

### 4.1 架构决策日志（task-centric draft-2）

> 起草自 [corivo-architecture-v2-spec.md](corivo-architecture-v2-spec.md)。这些是结构性架构决定，与 Part 2 的产品定位（Mirror + Coach）兼容；**具体落地节奏仍受 Part 3 的 alpha 用户反馈控制 —— 决策成立不等于现在就动手**。

- **D-T1**：task 升级为骨架，project 降级为可选标签，删除 projects 表与 P/T/E 三层结构
- **D-T2**：tags 表用 namespace 区分语义（project / domain / topic）；同 namespace 内 unique(namespace, name)
- **D-T3**：identity_anchors 同时支持 tag 和 task 两种 owner，各自维护，不冲突
- **D-T4**：attribution 走 task-first：event 先决定并入哪个 active task，task 形成后再由 tag_assignor 打 tag
- **D-T5**：多 active task 并行是常态，不是异常
- **D-T6**：events 不加 kind 字段。"工作 / 休闲" 由 task 是否挂 namespace='project' 的 tag 表达
- **D-T7**：events 是 SSOT，task 是物化语义投影，time_segments 是派生时间投影，about_user 是横切 prior
- **D-T8**：合并 Memory + Knowledge 为 about_user 扁平表，kind 字段开放扩展
- **D-T9**：用户 override（move_event / change_tag）是高优先级写保护信号，触发 about_user distill
- **D-T10**：task 完成态 / orphan event 都是合法终态，不进 retrospective 反复重试

完整设计见 [corivo-architecture-v2-spec.md](corivo-architecture-v2-spec.md)。

### 4.2 ~ 4.5：TBD

---

## Part 5 · Open Questions

下列问题尚未决定，列出来防止假装确定：

1. ~~是否保留 `project` 概念，还是塌缩为"目标列表"？~~ — **resolved（§4.1 D-T1）**：塌缩为 namespace='project' 的可选 tag，与 domain / topic 等其他 namespace 并列。"用户声明目标"如何与 tag 体系互动，仍 open
2. Mirror 第一批要 surface 哪 3–5 类模式？需要先做用户访谈取候选
3. Coach 的提醒频率天花板（防 nag）—— 一天最多说几句、用什么节奏
4. ~~`event_attributor` 是改成"对照声明目标"还是仍然推理 project？过渡方案？~~ — **resolved（§4.1 D-T4）**：改名 task_attributor，走 task-first（event → 并入 active task / 开新 task），project 不再是 attribution 主路径；声明目标如何介入仍待 alpha 反馈
5. `task_narrator` / `retrospective_reviewer` 在新定位下保留多少？是否合并为单一 drift scanner？
6. Year 2 信号阈值：什么数据点说明可以爬到 Assistant？（DAU / 留存 / 主动触发率？）
