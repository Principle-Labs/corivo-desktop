# Proactive Push — Stage C Roadmap

**创建于**：2026-04-18
**当前阶段**：B（只补"发现建议"阶段）
**下一阶段**：C（混合 GUMBO Mixed-Initiative + ContextAgent 主动性分数）

本文档**只记录 C 阶段要做什么、何时做**。B 阶段的正式实现计划另起 plan 文档（通过 brainstorming / writing-plans 流程产出）。

---

## 1. 背景

Corivo 当前的推送机制只有一条路径：`PropositionRevised` → `push_decider` 四道硬 gate（min_confidence / base_cooldown / dismiss_penalty / push_on_contradict）→ 弹 overlay。这**不是真正的 proactive assistant**——它推的是"命题被修订了"这个事件，而不是"此刻对用户有用的建议"。

参考论文：
- **GUM + GUMBO**（[arXiv:2505.10831](https://arxiv.org/abs/2505.10831)）——§4.3 Discovering Suggestions / Deciding When to Help / Feedback Loop
- **ContextAgent**（[arXiv:2505.14668](https://arxiv.org/abs/2505.14668)）——§4.2 主动性分数 P_S（1-5 级）+ SFT-CoT

## 2. B 阶段（当前做的，不是本文档的内容）

只做一件事：**加 `suggestion_generator` service**，把推送 payload 从"命题文本"改成"可执行建议文本"。

- `PropositionRevised` 触发 → 用 GUM query 检索相关命题集合 G → LLM 用 `新命题 + G + raw observations` 生成 N 条建议 → 选择最佳一条作为通知 payload
- **不动** `push_decider` 的四道硬 gate
- **不动** 反馈回流
- 目的：让 overlay 推的是"建议"而非"知识"；积累真实 `notification_log` outcome 数据为 C 阶段铺路

## 3. C 阶段目标

B 阶段跑通后，用真实数据驱动推送决策的升级。核心要引入的三样东西：

### 3.1 主动性分数（替换硬 gate）

采 **ContextAgent 式的 P_S 1-5 级分数**，不采 GUMBO 原版的 E[U] = P·B − (1−P)·C_FP 公式。原因：

- Mixed-Initiative 公式要求 LLM 同时吐出 B / C_FP / C_FN 三个数字，校准困难
- P_S 是单一整数输出，prompt 更简洁、评估更直接
- 可以和 suggestion_generator **合并成一次 LLM 调用**（产出建议 + 分数）

门槛：`P_S ≥ θ` 才推；`θ` 默认 3（配置可调）。分数 1、2 绝不推。

现有的 `min_confidence` / `push_on_contradict` 等硬 gate 在 C 阶段**保留为安全底线**（前置过滤），但 P_S 成为主决策。`base_cooldown` / `dismiss_penalty` 可能降级或合并进下面的 token-bucket。

### 3.2 全局 token-bucket 速率限制

GUMBO 原版：1 条/分钟全局上限。Corivo 的 `base_cooldown` 是 per-revision-group（7 天同组不重推），语义不同——两者可以共存：

- **token-bucket**：全局刷屏防护（默认 1/min，配置可调）
- **base_cooldown**：同一命题主题不骚扰（保留）

### 3.3 反馈回流 GUM

当前 `notification_log.outcome` 只影响 `dismiss_penalty`。GUMBO 的做法：

> "Gumbo simply converts the feedback into a text representation (*User disliked the following suggestion: [suggestion]*) and feeds it back into GUM. We treat feedback the same as any other unstructured observation."

要在 C 阶段加一条回流：`set_outcome(dismissed)` → 生成一条合成 observation 写入 `observations` 表 → 下次 PROPOSE/SIMILAR/REVISE 会看到。语义反馈进入命题模型，不再只是冷却惩罚。

## 4. 评估指标（从 ContextAgent §5.1 借鉴）

B 阶段靠定性观察；C 阶段开始要有量化指标：

| 指标 | 定义 | 目标 |
|------|------|------|
| **Acc-P** | 预测 P_S ≥ θ 与真值是否一致 | ≥ 80% |
| **MD**（missed detection） | 应推未推的比例 | ≤ 15% |
| **FD**（false detection） | 不该推却推的比例 | ≤ 10% |
| **RMSE** | 预测 P_S 与真值的均方根误差 | ≤ 1.0 |

ground truth 来源：用户 `notification_log.outcome`（acknowledged = 该推；dismissed = 不该推；ignored = 中性，暂不计入）。

## 5. 触发 C 阶段开发的条件

B 跑通后不立刻进入 C。触发条件（任一满足即可考虑启动 C）：

- `notification_log` 累计 outcome ≥ **500 条**（有足够数据评估硬阈值的准召）
- B 阶段的 dismiss 率 > **40%**（说明硬阈值误打扰严重，需要 P_S 替代）
- 用户主动反馈 B 阶段推送质量不够主动/或过于打扰

达到上述任一条件后，对着本文档走 brainstorming → writing-plans 流程，产出正式的 C 阶段实施 plan。

## 6. 开放问题（C 阶段设计时再回答）

- **P_S 由哪个 prompt 产出**？和 suggestion_generator 合并还是独立调用？
- **θ 是全局常量还是按时间段 / 应用上下文动态**？ContextAgent 论文 θ 是用户可调参数
- **反馈回流的合成 observation 文本格式**？是否要打 `feedback_source=true` 标签避免和真屏幕 observation 混淆
- **token-bucket vs base_cooldown 冲突时谁优先**？
- **prompt 从哪里蒸馏**？ContextAgent 用 Claude 3.7 Sonnet 蒸馏 CoT，Corivo 目前用 Gemini / Codex，要不要切换主力？

## 7. 参考

- [arXiv:2505.10831](https://arxiv.org/abs/2505.10831) — GUM & GUMBO，§4.3 Proactive Assistant
- [arXiv:2505.14668](https://arxiv.org/abs/2505.14668) — ContextAgent，§4.2 Context Reasoner / §5 评估
- 本项目当前实现：[push_decider.rs](../../../src-tauri/src/services/push_decider.rs) / [push_pipeline.rs](../../../src-tauri/src/services/push_pipeline.rs)
