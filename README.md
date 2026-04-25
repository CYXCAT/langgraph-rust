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
- [x] Phase 6：SQLite / Postgres checkpointer 及跨后端一致性测试
- [x] Phase 7 第一段：`langgraph-prebuilt` 最小闭环（`ToolNode` / `ValidationNode`）
- [x] Phase 7 第二段：`ToolNode` 高级能力（路由策略 / 错误策略 / 审计钩子）
- [x] Phase 7 第三段：`ValidationNode` 结构化规则集 + 可组合验证器 + 最小 `ReAct` 编排 API
- [x] Phase 7 第四段：默认 `planner/tool-calling` 模板 API（基于 `ReactAgentBuilder` 封装）
- [x] Phase 7 第五段：默认模板多工具策略（主工具 + 选择器 + 回退工具）
- [x] Phase 8 第一段：最小 `langgraph-cli`（JSON 输入 -> 执行 -> JSON 输出）

## 目录

- `crates/langgraph-core`：图模型、状态容器、错误
- `crates/langgraph-pregel`：Pregel 风格 superstep 执行器（同步 + 异步入口）
- `crates/langgraph-checkpoint`：checkpoint trait 与内存实现
- `crates/langgraph-prebuilt`：预置高层节点（`ToolNode` / `ValidationNode` / `react` / `react_template`）
- `crates/langgraph-cli`：最小命令行骨架（`run` 子命令，支持 stdin/文件输入与 JSON 输出）
- `tests/compat`：跨语言行为对照样例

## 下一步

1. 为 `langgraph-cli` 补充更多图预设与输入校验提示（提升可用性）
2. 为 prebuilt 新能力补充示例文档（模板接入、策略组合、验证报告消费方式）
3. 评估 `ToolNode`/`ValidationNode`/`react_template` 在流式事件中的可观测字段标准化
