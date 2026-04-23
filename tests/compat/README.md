# compat tests

该目录用于沉淀 Python LangGraph 到 Rust 的行为对照样例（golden fixtures）。

第一批样例建议覆盖：

- 线性图执行
- 分支与汇聚
- reducer 聚合
- checkpoint 恢复

命名约定：

- `*_input.json`：执行输入
- `*_expected.json`：期望输出
