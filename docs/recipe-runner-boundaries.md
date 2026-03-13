# Recipe Runner 的边界

这篇文档面向平台开发者，不面向普通 Recipe 使用者。

目标：
- 统一 `Recipe Source -> ExecutionSpec -> runner -> backend` 的分层理解
- 明确 runner 应该负责什么、不应该负责什么
- 约束何时新增业务动作，何时复用底座动作

## 1. 先定义 4 层

### Recipe Source

也就是作者写的 `recipe.json`。

它负责表达：
- 用户要填写什么参数
- 这条 recipe 想达成什么业务结果
- 应该被编译成哪些 action
- 结果文案如何展示

它不负责：
- 目标环境上的具体命令行细节
- 本地与远端执行差异
- 执行顺序里的低层物化细节

### ExecutionSpec

这是 Recipe DSL 的中间表示。

它负责表达：
- action 列表
- capability 使用
- resource claim
- execution kind
- source metadata

它不负责：
- 直接执行命令
- 直接做 UI copy

### runner

runner 是执行后端，不是通用脚本解释器。

它负责：
- 把 action 物化成 OpenClaw CLI、配置改写或内部底座命令
- 按目标环境路由到 `local`、`docker_local`、`remote_ssh`
- 执行前做必要的引用检查、环境准备和 fallback
- 产出 runtime run、artifacts、warnings

它不负责：
- 解释任意 shell 脚本
- 执行未经白名单声明的新 action
- 作为通用文件管理器处理二进制资源

### backend

backend 是 runner 最终调用的能力来源。

优先级固定为：
1. OpenClaw CLI / OpenClaw config 原语
2. ClawPal 的受控内部底座能力

## 2. OpenClaw-first 原则

这是当前 runner 的首要设计原则：

- 能用 OpenClaw 原语表达的动作，必须优先走 OpenClaw
- 只有 OpenClaw 暂时没有表达能力的资源，才允许 ClawPal fallback

当前典型映射：
- `create_agent` -> OpenClaw CLI
- `bind_agent` / `unbind_agent` -> OpenClaw CLI
- `set_agent_identity` -> OpenClaw CLI
- `set_channel_persona` / `clear_channel_persona` -> OpenClaw config rewrite
- `ensure_model_profile` / `ensure_provider_auth` -> 复用现有 profile/auth 同步能力
- `upsert_markdown_document` / `delete_markdown_document` -> ClawPal fallback
- `set_agent_persona` / `clear_agent_persona` -> 当前基于文档底座实现

这个原则的目的：
- 最大程度复用 OpenClaw
- 降低未来兼容性风险
- 避免把 Recipe 系统做成第二套 OpenClaw 配置内核

对 `create_agent` 还有一条额外约束：
- workspace 策略由 OpenClaw 决定
- 由于 `agents add --non-interactive` 需要显式 `--workspace`，runner 只会传入当前实例解析出的 OpenClaw 默认 workspace
- runner 不再为新 agent 推导 `--workspace <agent_id>` 这类 ClawPal 自定义路径
- 旧 source 里如果仍带 `independent`，当前只做兼容解析，不再影响 workspace 结果

## 3. 为什么不支持任意 shell

runner 刻意不支持：
- 任意 shell action
- 任意脚本片段
- 任意命令白名单外执行

原因很直接：
- 无法稳定推导 capability 和 resource claim
- 无法给非技术用户做可理解的 Review/Done 语义
- 无法做合理的风险控制、回滚和审计
- 会把 Recipe 降级成“远程脚本执行器”

如果一个需求只能靠通用 shell 才能表达，优先问两个问题：
1. 这是不是应该先成为 OpenClaw 原语？
2. 这是不是应该先成为受控的业务动作或底座动作？

## 4. action 白名单

当前 Recipe DSL 的 action surface 分两层主路径，再加两组底座/兼容动作。

### 推荐的业务动作

- `create_agent`
- `delete_agent`
- `bind_agent`
- `unbind_agent`
- `set_agent_identity`
- `set_agent_model`
- `set_agent_persona`
- `clear_agent_persona`
- `set_channel_persona`
- `clear_channel_persona`

### CLI 原语动作

这层按 OpenClaw CLI 子命令 1:1 暴露，适合高级 recipe 或只读检查 recipe。

当前 catalog 覆盖：
- `agents`
- `config`
- `models`
- `channels`
- `secrets`

例子：
- `list_agents`
- `show_config_file`
- `get_config_value`
- `models_status`
- `list_channels`
- `audit_secrets`

完整列表见：[recipe-cli-action-catalog.md](./recipe-cli-action-catalog.md)

### 文档动作

- `upsert_markdown_document`
- `delete_markdown_document`

### 环境动作

- `ensure_model_profile`
- `delete_model_profile`
- `ensure_provider_auth`
- `delete_provider_auth`

### 兼容 / escape hatch

- `config_patch`
- `setup_identity`
- `bind_channel`
- `unbind_channel`

新增 action 之前，先确认它不能被：
- 推荐的业务动作
- CLI 原语动作
- 文档动作
- 环境动作
合理表达。

## 5. 什么时候新增业务动作

优先新增业务动作，而不是继续堆 `config_patch`，当且仅当：

- 这个意图会反复出现在用户故事里
- 它对非技术用户来说有清晰结果语义
- 它值得单独审计、单独展示 Review/Done copy
- 它对应的 capability / claim 可以稳定推导

例如：
- `set_channel_persona` 比直接写 `config_patch` 更合适
- `set_agent_model` 比让 recipe 自己拼 config path 更合适
- `set_agent_identity` 比继续依赖 legacy `setup_identity` 更合适

## 6. 什么时候复用文档动作

优先复用 `upsert_markdown_document` / `delete_markdown_document`，当：

- 目标是文本/markdown 资源
- OpenClaw 暂时没有专门原语
- 需要 whole-file replace 或 section upsert
- 需要 local / remote 上一致的路径解析与写入语义

当前文档动作的目标范围是：
- `scope=agent`
- `scope=home`
- `scope=absolute`

但仍有限制：
- 只处理文本/markdown
- 相对路径里禁止 `..`
- `scope=agent` 必须能解析到合法 agent 文档目录

## 7. destructive 动作的默认阻断

第一阶段就支持 destructive action，但默认是保守的。

### `delete_agent`

默认会检查该 agent 是否仍被 channel binding 引用。

如果仍被引用：
- 默认失败
- 显式 `force=true` 或 `rebindChannelsTo` 才允许继续

### `delete_model_profile`

默认会检查该 profile 是否仍被 model binding 引用。

如果仍被引用：
- 默认失败

### `delete_provider_auth`

默认会检查该 authRef 是否仍被 model binding 间接使用。

如果仍被引用：
- 默认失败
- 显式 `force=true` 才允许继续

这套规则的目标不是“禁止删除”，而是让 destructive 行为必须有明确意图。

## 8. secret 与环境动作的边界

Recipe 不应携带明文 secret。

环境动作的原则：
- Recipe 只能引用现有 profile/auth/provider 关系
- 如果目标环境缺少依赖，runner 可以同步 ClawPal 已能解析到的 secret/auth
- secret 本体不应出现在 recipe params 或 source 里

换句话说：
- `ensure_model_profile` 可以触发 profile + auth 的准备
- 但 recipe source 自己不应成为 secret 载体

## 8.1 信任与批准不属于 runner 的“可选增强”

当前平台把来源信任和批准当成执行边界，而不是单纯 UI 提示。

来源分级：

- `bundled`
- `localImport`
- `remoteUrl`

runner / command layer 必须配合上层保证：

- 高风险 bundled recipe 未批准时不能执行
- 本地导入 recipe 在需要批准时不能执行
- 远程 URL recipe 的 mutating 行为未批准时不能执行

批准绑定到 `workspace slug + recipe digest`：

- digest 不变，批准可复用
- digest 变化，批准立即失效

这也是为什么 bundled recipe 升级不能静默覆盖：

- 一旦 source 变化，之前的批准就不再可信
- 用户需要明确看见新版本，并重新决定是否接受

## 9. Review / Done 为什么要依赖 action 语义

当前 UI 面向非技术用户，因此：
- Review 要展示“会得到什么结果”
- Done 要展示“已经完成了什么”
- Orchestrator 要展示“最近发生了什么效果”

如果 action 只有低层技术含义，例如裸 `config_patch`，UI 就只能暴露路径和技术细节。

因此，业务动作的价值不仅是执行方便，更是：
- 可翻译成自然语言
- 可推导影响对象
- 可生成稳定的结果文案

## 10. 何时应该修改 OpenClaw，而不是扩 runner

当一个需求满足下面任意一条时，应优先考虑给 OpenClaw 增加原语，而不是在 runner 里继续堆 fallback：

- 它已经是 OpenClaw 的核心资源模型
- 它需要长期稳定的 CLI/配置兼容承诺
- 它不是单纯的文本资源写入
- 它跨多个客户端都应该共享同一套语义

runner 适合作为：
- OpenClaw 原语的编排层
- OpenClaw 暂时缺位时的受控 fallback

runner 不适合作为：
- 一套长期独立于 OpenClaw 的第二执行内核

## 11. 设计新增 action 的最小检查表

新增一个 action 前，至少回答这几个问题：

1. 这个动作是业务动作、文档动作，还是环境动作？
2. 能否直接复用已有 action？
3. 能否优先映射到 OpenClaw？
4. 它需要哪些 capability？
5. 它会触碰哪些 resource claim？
6. 它是否是 destructive？
7. 它的 Review copy 和 Done copy 应该怎么表达？
8. 它是否需要默认阻断或引用检查？

如果这些问题答不清楚，不要先写 runner。

## 12. 关于 CLI 原语动作的边界

不是每个出现在 OpenClaw CLI 文档里的子命令，都适合直接由 Recipe runner 执行。

当前 catalog 会把它们分成两类：
- `runner supported = yes`
- `runner supported = no`

典型不能直接执行的情况：
- interactive 命令
- 需要明文 token / secret payload 的命令
- provider-specific flags 还没有稳定 schema 的命令

这些命令仍然会记录在 catalog 里，原因是：
- 文档和实现保持同一个事实源
- 作者能明确知道“这个 CLI 子命令存在，但当前不能写进 recipe”

## 13. 相关文档

- 作者指南：[recipe-authoring.md](./recipe-authoring.md)
- CLI catalog：[recipe-cli-action-catalog.md](./recipe-cli-action-catalog.md)
