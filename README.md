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
- [x] Phase 4 第一段：`Command::Interrupt` + runtime context 注入（兼容旧 `add_node`）
- [x] Phase 4 第二段：节点触发中断时自动持久化 checkpoint（可直接 resume）
- [x] Phase 4 第三段：`Command::Goto` + `add_conditional_edges` 条件路由衔接
- [x] Phase 4 第四段：组合命令（`GotoMany`/命令链）与 command trace 可观测性
- [x] Phase 4 第五段：命令策略层（`CommandPolicy`）+ bridge 审计钩子
- [x] Phase 5 第一段：异步入口（`ainvoke` / `aresume` 等）与同步语义对齐测试
- [x] Phase 5 第二段：`astream` 事件流模型（节点事件 / 状态片段 / 命令与中断事件）
- [x] Phase 5 第三段：checkpoint bridge thread 维度 `astream`（流式执行 + 自动落盘 + resume 闭环）

## 目录

- `crates/langgraph-core`：图模型、状态容器、错误
- `crates/langgraph-pregel`：Pregel 风格 superstep 执行器（同步 + 异步入口）
- `crates/langgraph-checkpoint`：checkpoint trait 与内存实现
- `tests/compat`：跨语言行为对照样例

## 下一步

1. 继续 Phase 5：细化流式事件协议（完成事件、错误事件、可选过滤器）
2. 进入 Phase 6：实现 SQLite checkpointer 与基础 schema 迁移
3. 扩展 Phase 6：补齐 Postgres checkpointer 与集成测试
