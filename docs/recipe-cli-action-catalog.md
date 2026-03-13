# Recipe CLI Action Catalog

这篇文档是 Recipe DSL 的高级参考，面向：
- 需要直接复用 OpenClaw CLI 原语的 recipe 作者
- 维护 runner/action catalog 的平台开发者

普通业务 recipe 请先看：[recipe-authoring.md](./recipe-authoring.md)。

## 1. 设计规则

- 一个 CLI 原语动作尽量对应一个 OpenClaw CLI 子命令
- `Runner supported = yes` 表示当前 Recipe runner 可以直接执行
- `Runner supported = no` 表示该动作只记录在 catalog 中，当前不能由 Recipe runner 执行
- `Recommended direct use = no` 表示虽然能执行，但更推荐用高层业务动作

## 2. Agents

| DSL action | OpenClaw CLI | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `list_agents` | `openclaw agents list` | yes | no | 只读检查动作 |
| `list_agent_bindings` | `openclaw agents bindings` | yes | no | 只读检查动作 |
| `create_agent` | `openclaw agents add` | yes | yes | 推荐业务动作；runner 只会传入当前实例解析出的 OpenClaw 默认 workspace，不再使用 `agent_id` 这类自定义路径 |
| `delete_agent` | `openclaw agents delete` | yes | yes | 会先做 binding 引用检查 |
| `bind_agent` | `openclaw agents bind` | yes | yes | 推荐替代旧 `bind_channel` |
| `unbind_agent` | `openclaw agents unbind` | yes | yes | 支持 `binding` 或 `all=true` |
| `set_agent_identity` | `openclaw agents set-identity` | yes | yes | 推荐替代旧 `setup_identity` |

## 3. Config

| DSL action | OpenClaw CLI | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `show_config_file` | `openclaw config file` | yes | no | 只读检查动作 |
| `get_config_value` | `openclaw config get` | yes | no | 只读检查动作 |
| `set_config_value` | `openclaw config set` | yes | no | 可直接写值；大多数业务 recipe 优先用业务动作 |
| `unset_config_value` | `openclaw config unset` | yes | no | 同上 |
| `validate_config` | `openclaw config validate` | yes | no | 只读检查动作 |
| `config_patch` | 多条 `openclaw config set` | yes | no | escape hatch，不是 1:1 CLI 子命令 |

## 4. Models

| DSL action | OpenClaw CLI | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `models_status` | `openclaw models status` | yes | no | 支持 probe 相关 flags |
| `list_models` | `openclaw models list` | yes | no | 只读检查动作 |
| `set_default_model` | `openclaw models set` | yes | no | 会改默认模型，不会改指定 agent |
| `scan_models` | `openclaw models scan` | yes | no | 只读检查动作 |
| `list_model_aliases` | `openclaw models aliases list` | yes | no | 只读检查动作 |
| `list_model_fallbacks` | `openclaw models fallbacks list` | yes | no | 只读检查动作 |
| `add_model_auth_profile` | `openclaw models auth add` | no | no | provider-specific schema 还没收口 |
| `login_model_auth` | `openclaw models auth login` | no | no | interactive |
| `setup_model_auth_token` | `openclaw models auth setup-token` | no | no | interactive / token flow |
| `paste_model_auth_token` | `openclaw models auth paste-token` | no | no | 需要 secret payload，不应进 recipe source |
| `set_agent_model` | 编排动作 | yes | yes | 高层业务动作，优先使用 |
| `ensure_model_profile` | 编排动作 | yes | yes | 高层环境动作，优先使用 |
| `delete_model_profile` | 编排动作 | yes | yes | 高层环境动作，优先使用 |
| `ensure_provider_auth` | 编排动作 | yes | yes | 高层环境动作，优先使用 |
| `delete_provider_auth` | 编排动作 | yes | yes | 高层环境动作，优先使用 |

## 5. Channels

| DSL action | OpenClaw CLI | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `list_channels` | `openclaw channels list` | yes | no | 只读检查动作 |
| `channels_status` | `openclaw channels status` | yes | no | 只读检查动作 |
| `read_channel_logs` | `openclaw channels logs` | no | no | 目前还没定义稳定参数 schema |
| `add_channel_account` | `openclaw channels add` | no | no | provider-specific flags 太多，后续再抽象 |
| `remove_channel_account` | `openclaw channels remove` | no | no | 当前未抽象稳定 schema |
| `login_channel_account` | `openclaw channels login` | no | no | interactive |
| `logout_channel_account` | `openclaw channels logout` | no | no | interactive |
| `inspect_channel_capabilities` | `openclaw channels capabilities` | yes | no | 只读检查动作 |
| `resolve_channel_targets` | `openclaw channels resolve` | yes | no | 只读检查动作 |
| `set_channel_persona` | `openclaw config set` | yes | yes | 高层业务动作，优先使用 |
| `clear_channel_persona` | `openclaw config set` | yes | yes | 高层业务动作，优先使用 |

## 6. Secrets

| DSL action | OpenClaw CLI | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `reload_secrets` | `openclaw secrets reload` | yes | no | 只读/刷新动作 |
| `audit_secrets` | `openclaw secrets audit` | yes | no | 只读检查动作 |
| `configure_secrets` | `openclaw secrets configure` | no | no | interactive |
| `apply_secrets_plan` | `openclaw secrets apply --from ...` | yes | no | 高级动作，直接消费 plan 文件 |

## 7. Fallback / Document

这些动作不是 OpenClaw CLI 子命令，但仍然是 DSL 的正式组成部分：

| DSL action | Backend | Runner supported | Recommended direct use | Notes |
| --- | --- | --- | --- | --- |
| `upsert_markdown_document` | ClawPal document writer | yes | no | 仅限文本/markdown |
| `delete_markdown_document` | ClawPal document writer | yes | no | 仅限文本/markdown |
| `set_agent_persona` | ClawPal document writer | yes | yes | 当前还没有 OpenClaw 原语，所以保留 fallback |
| `clear_agent_persona` | ClawPal document writer | yes | yes | 同上 |
| `setup_identity` | legacy compatibility | yes | no | 旧动作，保留兼容 |
| `bind_channel` | legacy compatibility | yes | no | 旧动作，保留兼容 |
| `unbind_channel` | legacy compatibility | yes | no | 旧动作，保留兼容 |

## 8. 什么时候直接用 CLI 原语动作

适合直接用 CLI 原语动作的场景：
- 你要写只读检查 recipe
- 你要做平台维护/运维型 recipe
- 你明确需要 OpenClaw CLI 的精确语义

不适合的场景：
- 面向非技术用户的 bundled recipe
- 可以清楚表达成业务动作的配置改动
- 需要携带 secret payload 的命令
- interactive 命令

## 9. 相关文档

- 作者指南：[recipe-authoring.md](./recipe-authoring.md)
- Runner 边界：[recipe-runner-boundaries.md](./recipe-runner-boundaries.md)
