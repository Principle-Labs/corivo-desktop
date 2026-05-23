# Proposition Retrieval — FTS5 + Jieba 中文全文检索 Design

## 1. Goal

把命题检索（[`src-tauri/src/db/repos/propositions.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/propositions.rs) 里的 `query_like`）从"整段 LIKE 子串匹配"换成"SQLite FTS5 全文索引 + 中文 jieba 分词"。让 `services::user_model::retrieval::query` 在召回阶段不再因为措辞差异漏掉语义相关的命题，从而让 PROPOSE → SIMILAR → REVISE 流水线真正有机会触发 `merge` / `update`。

这次改动只动**召回（候选池构建）这一层**。后面的 SIMILAR / REVISE prompt、加权打分公式、push_decider 全部不动。

## 2. Why This Change

当前 [`db/repos/propositions.rs:240`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/propositions.rs#L240) 的 `query_like`：

```rust
let like_pattern = args.text.as_deref().map(|q| format!("%{q}%"));
// SQL: WHERE p.text LIKE ?1 OR p.reasoning LIKE ?1
```

整段 draft 文本被 `%...%` 包起来做 SQL `LIKE`——这是**连续子串**匹配。对中文写作而言，两条措辞不同但语义重叠的命题几乎不可能互为子串。例如：

```
A: 用户处理事务时倾向于先关注沟通信息，再转入正式文档进行细致核对和编辑。
B: 用户会及时查看并处理工作沟通中的新消息反馈。
```

A 不是 B 的子串、B 也不是 A 的子串。任意一条作为 draft 进 [`build_similarity_pool`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/proposition_pipeline.rs#L58) 时，对方都不会出现在候选池里。SIMILAR 阶段拿不到，REVISE 阶段更拿不到——所以两条命题永远各自存在，"合并力度不够"的根因正是这里。

更广泛影响：

- `merge` 永远不会被触发，因为它要求**同一 draft 同时撞到 ≥2 条现有命题**
- `update` / `rewrite` / `contradict` 触发率极低，命题表会无界增长
- [`retrieval.rs:138`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/retrieval.rs#L138) 的 `match_bonus` 也跟着失效，加权排序退化成"只看 confidence × decay"

英文场景下 `LIKE` 至少能撞到关键词；中文没有空格分词，必须显式做分词才能召回。

## 3. Non-Goals

- 不动 PROPOSE / SIMILAR / REVISE 的 prompt 文本
- 不动 [`retrieval.rs::score_proposition`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/retrieval.rs#L109) 加权打分公式
- 不动 push_decider 的判定逻辑
- 不引入向量嵌入（`embedding` / `cosine similarity`）——属于另一份 spec
- 不做 jieba 自定义词典加载（默认词典够用，自定义词典留 P1）
- 不引入异步后台"存量合并"任务（merge 仍然由新 draft 拉动；提升召回率本身就足以让"两条同时落入 pool"的概率从 ~0 升到可观）

## 4. Chosen Tokenization Path

### 4.1 两个候选实现

**方案 A：自定义 FTS5 tokenizer**
用 `jieba-rs` 实现 FTS5 的 `fts5_tokenizer` C 接口，把 `"jieba"` 注册为 SQLite 可识别的 tokenizer。FTS5 自动在写入和查询时都调用它。

- 优点：写入/查询都由 SQLite 调度，应用侧只发原文
- 缺点：rusqlite 0.31 没有现成的 FTS5 自定义 tokenizer 安全包装，得自己写 unsafe FFI；每次 `Connection` 打开都要重新注册（r2d2 pool 的 `connection_customizer` 钩子里挂），出错风险高

**方案 B：预分词 + FTS5 默认 tokenizer**
应用侧用 `jieba-rs` 把 `text + reasoning` 切成空格分隔的 token 字符串，写到 FTS5 表的 `tokens` 列。FTS5 用默认 `unicode61` tokenizer 直接按空格切就好。查询时同样用 jieba 切 query，再拼成 FTS5 MATCH 表达式。

- 优点：纯应用侧逻辑、可单测、可观察（中间态 token 字符串可以直接 SELECT 出来肉眼审）；切换 jieba 模式（`cut` / `cut_for_search`）只改一处；将来加自定义词典也只改一处
- 缺点：FTS 索引和源表必须由应用侧手动保持同步，绕过 repo 直写 SQL 会导致索引漂移

### 4.2 决策：方案 B

理由：

1. **工程量小**——不碰 SQLite C 接口，不动 r2d2 connection customizer
2. **可调试**——`SELECT tokens FROM propositions_fts WHERE rowid = ?` 能直接看到分词结果
3. **职责清晰**——分词策略完全在 Rust 侧，FTS5 只负责倒排
4. **未来兼容**——若将来要切方案 A 也只是把"应用侧切词 + 查 FTS"换成"FTS 自动切词"，倒排表结构不变

## 5. Schema Changes (V6)

新增迁移 `src-tauri/src/db/migrations/006_propositions_fts.sql`：

```sql
-- FTS5 contentless 表，rowid 与 propositions.id 对齐。
-- contentless（content=''）意味着 FTS5 不存原文，只存倒排索引。
-- 应用读到 rowid 后回 propositions 主表 JOIN 取完整字段。
CREATE VIRTUAL TABLE propositions_fts USING fts5(
    tokens,
    content='',
    tokenize='unicode61 remove_diacritics 0'
);

INSERT OR IGNORE INTO schema_version (version) VALUES (6);
```

设计点说明：

- **contentless 表**：节省一份正文存储，且杜绝"FTS 拷贝和主表漂移"——FTS 只是个倒排索引
- **`unicode61 remove_diacritics 0`**：默认 tokenizer 按 Unicode 类别切；`remove_diacritics 0` 关掉变音符号归一化，对中文无影响但避免某些拉丁字符意外被合并
- **不加 SQL 触发器**：分词必须在 Rust 侧做，trigger 里没法调 jieba。同步责任由 repo 承担

`apply_v6` 在 [`db/migrations.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/migrations.rs) 里：先跑上面的 SQL，再立刻**回填存量命题的 FTS 索引**——遍历 `propositions` 全表，每行调 `tokenize::tokenize_for_index(text + reasoning)`，写到 `propositions_fts (rowid, tokens)`。回填失败不阻塞迁移，但要 `tracing::warn!` 出来；下次启动如果发现有 `propositions.id` 缺失对应 FTS 行，可以补回填。

更新 `LATEST_VERSION = 6`，`SCHEMA_V6` 用 `include_str!` 引入。

## 6. Tokenization Module

新增 [`src-tauri/src/services/user_model/tokenize.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/tokenize.rs)：

```rust
use jieba_rs::Jieba;
use once_cell::sync::Lazy;

static JIEBA: Lazy<Jieba> = Lazy::new(Jieba::new);

/// 入库分词：精确模式，输出空格拼接的 token 串。
/// 适合"长期、稳定"的索引内容——切粒度适中，避免冗余倒排。
pub fn tokenize_for_index(text: &str) -> String {
    JIEBA.cut(text, /* hmm = */ true)
        .into_iter()
        .filter(|tok| is_meaningful(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 查询分词：搜索引擎模式（cut_for_search），切得更细以提升召回。
/// 输出已经做过 FTS5 保留字符过滤的 MATCH 表达式（OR 连接）。
/// 例：`沟通 OR 消息 OR 反馈`
pub fn tokenize_for_query(text: &str) -> Option<String> {
    let tokens: Vec<String> = JIEBA
        .cut_for_search(text, true)
        .into_iter()
        .filter(|tok| is_meaningful(tok))
        .map(sanitize_for_match)
        .filter(|tok| !tok.is_empty())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" OR "))
    }
}

fn is_meaningful(tok: &str) -> bool {
    let t = tok.trim();
    !t.is_empty()
        && t.chars().any(|c| c.is_alphanumeric() || !c.is_ascii())
}

/// 转义/丢弃 FTS5 MATCH 语法中的保留字符：
/// `"` `(` `)` `*` `^` `:` 以及裸的 `OR` `AND` `NOT` `NEAR`
fn sanitize_for_match(tok: String) -> String {
    // 见 §10.3 风险节
    todo!()
}
```

只暴露这两个函数 + `JIEBA` 单例。其它模块不直接 `use jieba_rs`。

## 7. Repo Surface Changes

### 7.1 新增 `query_fts`，替换 `query_like`

[`db/repos/propositions.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/propositions.rs) 的 trait 方法 `query_like` 改名为 `query_fts`，签名 `QueryLikeArgs` 改名为 `QueryFtsArgs`（字段不变）。实现：

```rust
async fn query_fts(&self, args: QueryFtsArgs) -> Result<Vec<Proposition>> {
    let match_expr = match args.text.as_deref().and_then(tokenize_for_query) {
        Some(expr) => expr,
        None => return Ok(Vec::new()), // 空查询直接返回空，避免 MATCH '' 报错
    };
    let start = args.start_time.as_ref().map(datetime_to_sql);
    let end = args.end_time.as_ref().map(datetime_to_sql);
    let pool_limit = args.pool_limit;
    run_blocking(self.pool.clone(), move |conn| {
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM propositions_fts f
             JOIN propositions p ON p.id = f.rowid
             WHERE f.tokens MATCH ?1
               AND (?2 IS NULL OR p.created_at >= ?2)
               AND (?3 IS NULL OR p.created_at <= ?3)
             ORDER BY COALESCE(p.confidence, 0) DESC
             LIMIT ?4"
        );
        // ...
    })
    .await
}
```

注意：`SELECT_COLUMNS` 现有常量已经包含完整列；`p.` 前缀要补齐。`MATCH` 表达式直接绑参（rusqlite 会安全 escape 字符串字面量；FTS5 的语法字符已经在 `sanitize_for_match` 里处理过）。

### 7.2 写入路径同步 FTS

`PropositionRepo::insert` 现在的实现是单条 INSERT。改成在同一个 `run_blocking` 闭包里做事务：

```sql
BEGIN;
INSERT INTO propositions (...) VALUES (...);
INSERT INTO propositions_fts (rowid, tokens) VALUES (last_insert_rowid(), ?);
COMMIT;
```

`tokens` 由 Rust 侧调 `tokenize_for_index(format!("{text}\n{reasoning}"))` 算好后传入。

`update_revision_group` 和 `set_contradicts` 都不修改 `text` / `reasoning`，**不需要**更新 FTS。如果将来加了"修改命题正文"的接口，必须配套 `DELETE FROM propositions_fts WHERE rowid = ?` + `INSERT`。

### 7.3 调用点替换

[`services/user_model/retrieval.rs::query`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/retrieval.rs#L49) 把 `repo.query_like(...)` 改成 `repo.query_fts(...)`。其他地方不变。`match_bonus` 计算保留——它现在能更经常命中，因为命题终于会被召回了。

### 7.4 Mock / Stub 更新

[`db/repos/propositions.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/db/repos/propositions.rs) 的 `InMemoryPropositionRepo`（如有）以及测试桩需要同步实现 `query_fts`——可以简单复用现有 `query_like` 的子串语义；reuse 不影响行为正确性，只影响性能。

## 8. Dependency Changes

`src-tauri/Cargo.toml`：

```toml
[dependencies]
# 已有
rusqlite = { version = "0.31", features = ["bundled", "chrono", "serde_json"] }

# 新增
jieba-rs = "0.7"     # MIT，纯 Rust，无 C 依赖
once_cell = "1"      # 若已有则跳过
```

`rusqlite` 的 `bundled` 已经默认编译进 FTS5（SQLite 3.40+ 默认开启 `SQLITE_ENABLE_FTS5`）。无需额外 feature。

## 9. Testing Strategy

### 9.1 Rust 单测

新增 `src-tauri/src/services/user_model/tokenize.rs` 内联 `#[cfg(test)]`：

- `tokenize_for_index`：固定中文输入 → 期望 token snapshot
- `tokenize_for_query`：固定输入 → 期望 MATCH 表达式 snapshot
- `sanitize_for_match`：覆盖 `"` `(` `)` `*` `^` `:` 以及 `OR` 等保留字
- 边界：纯空白、纯标点、纯英文、中英混排、emoji

新增 `src-tauri/tests/proposition_fts_repo.rs`：

- 用真实 SqlitePool，写入两条用户原话命题
- query_fts("用户在工作沟通中的反馈") 应该同时召回 A 和 B
- 时间区间过滤
- 空 query 返回空结果（不报错）

### 9.2 集成回归

[`src-tauri/tests/proposition_pipeline_*.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/tests/) 现有用例必须保持 green。重点：

- PROPOSE → SIMILAR → REVISE 的端到端测试用的 mock LLM 不受分词改动影响
- `build_similarity_pool` 的覆盖率测试要补一个"两条措辞不同的同主题命题，第三条 draft 同时撞到两条"的用例，断言 REVISE 收到的 candidate 列表确实包含两条

### 9.3 迁移测试

新增 `src-tauri/tests/migrations_v6.rs`：

- 装一个 v5 schema 的库，预先塞 N 条 propositions
- 跑 `apply_migrations`，断言：
  - `schema_version` = 6
  - `propositions_fts` 表存在
  - 每条 propositions 都有对应的 FTS 行
  - 一条简单 MATCH 查询能命中

## 10. Risks & Mitigations

### 10.1 Jieba 词典首次加载耗时

`Jieba::new()` 加载内置词典约需 200–500 ms。

缓解：
- `Lazy<Jieba>` 全局单例，应用启动时在 setup 里主动 `Lazy::force(&JIEBA)` warm-up
- warm-up 放后台 task，不阻塞 UI 启动

### 10.2 FTS 索引和源表漂移

任何绕过 repo、直接 raw SQL 写 propositions 的代码路径都会让 FTS 不更新。

缓解：
- 全局 grep 确认没有除 repo 之外的 `INSERT INTO propositions`
- repo 加单测：`insert` 后立刻 `query_fts` 应该能命中
- v6 迁移结束加一次性自检：`SELECT count(*) FROM propositions != SELECT count(*) FROM propositions_fts` 时 `tracing::warn!`

### 10.3 FTS5 MATCH 语法注入

jieba 切出的 token 里如果含 `"` `(` `)` `*` `^` `:`，或者整 token 是 `OR` / `AND` / `NOT` / `NEAR`，会被 FTS5 当语法解析，轻则报错重则误命中。

缓解：`sanitize_for_match` 统一处理：
- 把每个 token 用双引号包成 phrase 字面量：`"沟通"` → 在 FTS5 里就是字面词
- 内部的 `"` 替换成 `""`
- 整体丢弃只含标点的 token

经此处理后，`OR` 连接符是我们手动拼的，不会和 token 内的 `OR` 混淆。

### 10.4 Bundled SQLite 版本

确认 `rusqlite = "0.31" + bundled` 的 SQLite 版本 ≥ 3.40（FTS5 + contentless 完全可用）。当前 0.31 bundled 是 3.45+，OK。后续升 rusqlite 时复查。

### 10.5 测试桩 InMemory Repo

如果 trait 有非数据库实现（mock），新加 `query_fts` 必须同步实现，否则测试会编译失败。这是发现遗漏调用点的好兜底。

## 11. Rollout Order

1. 加 `jieba-rs` + `once_cell` 依赖，跑通 `cargo build`
2. 加 [`tokenize.rs`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/tokenize.rs) + 单测
3. 加迁移 v6（SQL + Rust 回填 + `LATEST_VERSION` 改 6）+ 迁移测试
4. trait 加 `query_fts` 方法，所有实现补齐；保留 `query_like` 但 `#[deprecated]` 标记，让编译器提示调用点
5. 修改 `insert` 同步写 FTS，加事务包裹
6. 切换 [`retrieval.rs::query`](/Users/airbo/Developer/corivo/corivo-app/src-tauri/src/services/user_model/retrieval.rs#L49) 的调用点；删除 `query_like`（包括 trait 上的方法）
7. 跑全量 `cargo test`；补 §9.2 的回归用例
8. 启动 warm-up：`lib.rs::run` 的 setup 里 spawn 一个 `tokio::task::spawn_blocking(|| Lazy::force(&JIEBA))`

每步都能独立提交，且 5 之前的步骤不影响生产行为（只是新增表 + 新方法）。

## 12. Decision

采用方案 B：**应用侧 jieba 预分词 + FTS5 contentless 倒排表 + 迁移 v6 一次性回填**。

- 召回机制从"整段连续子串"升级为"按 token OR 召回"
- 写入路径单点同步 FTS，依赖 repo 封装保持一致性
- 不动加权评分、不动 LLM prompt、不动 push 链路——这次只解决"候选池根本捞不到"的根因
- 总改动量预计 1 份迁移 + 1 个新模块 + 1 个 repo 方法替换 + 1 个调用点切换
