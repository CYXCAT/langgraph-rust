# langgraph-rust

Rust 版 LangGraph 迁移的最小可运行骨架，原项目地址：https://github.com/langchain-ai/langgraph 。

## 当前进度

- [x] Cargo workspace 初始化
- [x] 创建 `langgraph-core`、`langgraph-pregel`、`langgraph-checkpoint`
- [x] 最小 `StateGraph` builder + `compile`
- [x] 顺序执行器 `invoke`
- [x] `{"text":"ab"}` 最小示例测试
- [x] 统一错误类型 `GraphError` 与 `StateValue`
- [x] `CheckpointSaver` trait 草案 + `InMemorySaver`
- [x] `tests/compat/` 第一批 fixture
- [x] channel 抽象（`LastValue` / `BinaryOperatorAggregate` / `Topic`）与 superstep 执行
- [x] `versions_seen` / channel versioning 元数据链路
- [x] checkpoint 恢复闭环（`resume` / `resume_latest` / `pending_writes` materialize）
- [x] 显式中断与恢复（`interrupt_thread`）
- [x] `resume_latest` 最新 checkpoint 选择修复（按数值版本，不依赖返回顺序）

## 目录

- `crates/langgraph-core`：图模型、状态容器、错误
- `crates/langgraph-pregel`：Pregel 风格 superstep 执行器（当前为同步实现）
- `crates/langgraph-checkpoint`：checkpoint trait 与内存实现
- `tests/compat`：跨语言行为对照样例

## 下一步

1. 进入 Phase 4：实现 `Command` / `interrupt` 语义与 runtime context 注入
2. 在 `langgraph-core` 增加 `add_conditional_edges` 与分支路由模型
3. 进入 Phase 5：补齐 async/streaming（`ainvoke` / `astream`）
