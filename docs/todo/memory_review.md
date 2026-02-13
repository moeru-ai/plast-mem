# Memory Review 重构 Plan

## Context

当前 retrieval 后自动执行 auto-GOOD review，无差别强化所有被检索的 memory。这会导致自我强化随机偏见——被错误检索的 memory 也会被强化，下次更容易被检索到。

重构目标：去掉 auto-GOOD，改为 segmentation 时由 LLM reviewer 评估 retrieved memory 的实际效果，用 Again/Hard/Good/Easy 更新 FSRS 参数。

核心原则：**review ≠ retrieval**。retrieval 是检索，review 是事后评估。只有 review 才更新 FSRS 参数（包括 last_reviewed_at）。

## 流程

```text
retrieve_memory(query, conversation_id)
  → 正常检索返回结果（不更新任何 FSRS 参数）
  → 在 message_queue 追加 pending review 信息：{ memory_ids, query }

对话继续（assistant msg, user msg, ...）

segmentation 触发时（rule 或 LLM 判断要 segment）：
  → 检查 message_queue 是否有 pending review
  → 如果有，enqueue MemoryReviewJob：
      context = 整段对话 messages
      retrieved_memory_ids
      queries
  → 清除 pending review 信息
  → 正常执行 segmentation（创建 episodic memory）

MemoryReviewJob（异步 worker）：
  → LLM 评估每个 retrieved memory 在对话 context 中的效果
  → 输出 Again/Hard/Good/Easy rating
  → 用 rating 更新 stability, difficulty, last_reviewed_at
```

## 改动清单

### 1. DB Migration：message_queue 表加列

- `pending_reviews`: `JSONB` (nullable, default null)
- 结构：`[{ memory_ids: UUID[], query: String }]`（数组，支持一段对话中多次检索）

### 2. retrieve_memory API 改动

文件：`crates/server/src/api/retrieve_memory.rs`

- 请求加 `conversation_id: Uuid` 参数
- 去掉 `enqueue_review_job()` 调用
- 改为：检索完成后，更新 message_queue 的 `pending_reviews` 列（追加当前检索的 memory_ids + query）

### 3. 去掉 retrieval 时的 auto-GOOD

文件：`crates/server/src/api/retrieve_memory.rs`

- 删除 `enqueue_review_job` 函数
- 删除两个 endpoint 中对它的调用

### 4. MessageQueue 扩展

文件：`crates/core/src/message_queue.rs`

- 添加 pending review 相关方法：
  - `add_pending_review(id, memory_ids, query, db)` — 追加一条 pending review
  - `take_pending_reviews(id, db)` — 取出并清除所有 pending reviews
  - `has_pending_reviews(id, db)` — 检查是否有 pending

### 5. Segmentation 流程改动

文件：`crates/worker/src/jobs/event_segmentation.rs`

- `process_event_segmentation` 中，在创建 episodic memory 之后（drain 之前）：
  - 调用 `MessageQueue::take_pending_reviews`
  - 如果有 pending reviews，enqueue `MemoryReviewJob`（带上整段对话 messages + pending review 信息）

### 6. MemoryReviewJob 重写

文件：`crates/worker/src/jobs/memory_review.rs`

- Job 结构改为：

```rust
pub struct MemoryReviewJob {
    pub reviews: Vec<PendingReview>,  // memory_ids + query per retrieval
    pub context_messages: Vec<Message>,  // 整段对话
    pub reviewed_at: DateTime<Utc>,
}
```

- `process_memory_review` 改为：
  1. 对每个 retrieved memory，调用 LLM 评估 relevance（输出 Again/Hard/Good/Easy）
  2. 用对应 rating 的 `next_states` 更新 stability/difficulty/last_reviewed_at
  3. 保留 stale skip 逻辑（reviewed_at <= last_reviewed_at → skip）

### 7. LLM Review 函数

文件：`crates/ai/src/lib.rs`（新增 `review_memories` 或类似模块）

- 输入：context messages + retrieved memory summaries + query
- 输出：每个 memory 的 rating (Again/Hard/Good/Easy)
- structured output，类似 segment_events 的模式

### 8. 文档更新

- `docs/architecture/fsrs.md`：更新 Review 章节，去掉 "Planned behavior" 标记
- `docs/architecture/retrieve_memory.md`：更新 Side Effects 章节，说明 retrieval 不再直接触发 review
- `AGENTS.md`：更新 Key Runtime Flows

## Review Rating 定义

| Rating | 含义 | FSRS 效果 |
| ------ | ---- | --------- |
| Again | memory 在对话中完全没被用上，是噪音 | stability 大幅下降 |
| Hard | memory 相关但需要推理才能关联 | stability 基本不变 |
| Good | memory 直接相关且有用 | stability 适度增长 |
| Easy | memory 是核心知识，频繁被检索且每次都直接相关 | stability 大幅增长 |
