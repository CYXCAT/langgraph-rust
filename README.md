# langgraph-rust

Rust 版 LangGraph 迁移的最小可运行骨架，原项目地址：https://github.com/langchain-ai/langgraph。

## 当前进度

- [x] Cargo workspace 初始化
- [x] 创建 `langgraph-core`、`langgraph-pregel`、`langgraph-checkpoint`
- [x] 最小 `StateGraph` builder + `compile`
- [x] 顺序执行器 `invoke`
- [x] `{"text":"ab"}` 最小示例测试
- [x] 统一错误类型 `GraphError` 与 `StateValue`
- [x] `CheckpointSaver` trait 草案 + `InMemorySaver`
- [x] `tests/compat/` 第一批 fixture

## 目录

- `crates/langgraph-core`：图模型、状态容器、错误
- `crates/langgraph-pregel`：顺序执行器（后续替换为 Pregel superstep）
- `crates/langgraph-checkpoint`：checkpoint trait 与内存实现
- `tests/compat`：跨语言行为对照样例

## 下一步

1. 在 `langgraph-core` 增加 `add_conditional_edges` 与分支模型
2. 在 `langgraph-pregel` 引入 channel 抽象和 superstep 调度
3. 为 checkpoint 增加 thread 多轮执行与恢复集成测试
