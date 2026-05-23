---
name: corivo-feedback
description: 当用户在对话中表达想反馈问题、报 bug、提建议或联系 Corivo 团队时使用。引导用户用本机已有的邮件方式（飞书邮箱、Gmail、Apple Mail 等）发送到 hi@corivo.ai。
---

# Corivo 用户反馈

当用户表达「我想反馈」「有 bug」「提建议」「联系团队」「找你们」等意图时使用。

## 目标邮箱

**hi@corivo.ai** — 这个邮箱直连 Corivo 后台的用户问题处理系统，邮件即工单。

## 流程

1. **挑用户机器上已有的邮件方式**，按优先级：
   - 飞书邮箱（Lark Mail）
   - Gmail / Apple Mail / Outlook / 其他桌面客户端
   - 浏览器里的网页邮箱
2. **如果用户没有邮件客户端**：让用户复制 `hi@corivo.ai`，用任何他熟悉的方式（手机邮箱、网页等）发到这个地址。
3. **帮用户起草邮件**：
   - 标题：`Corivo 反馈：<一句话主题>`
   - 正文：问题/建议描述、复现步骤（bug 类）、Corivo 版本和 macOS 版本（如果用户愿意提供）
4. **让用户在他熟悉的客户端里检查草稿、确认无误**。
5. 用户确认之后，可以通过 CLI、MCP 或其他可用工具（例如 lark-mail CLI、Gmail MCP、`open mailto:...` 等）实际触发发送 —— 走和用户确认过的同一条发送通路。
