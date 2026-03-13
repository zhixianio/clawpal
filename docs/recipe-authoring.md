# 如何编写一个 ClawPal Recipe

这份文档描述的是当前仓库里真实可执行的 Recipe DSL，而不是早期草案。

目标读者：
- 需要新增预置 Recipe 的开发者
- 需要维护 `examples/recipe-library/` 外部 Recipe 库的人
- 需要理解 `Recipe Source -> ExecutionSpec -> runner` 这条链路的人

## 1. 先理解运行时模型

当前 ClawPal 的 Recipe 有两种入口：

1. 作为预置 Recipe 随 App 打包，并在启动时 seed 到 workspace
2. 作为外部 Recipe library 在运行时导入

无论入口是什么，最终运行时载体都是 workspace 里的单文件 JSON：

`~/.clawpal/recipes/workspace/<slug>.recipe.json`

也就是说：
- source authoring 可以是目录结构
- import/seed 之后会变成自包含单文件
- runner 永远不直接依赖外部 `assets/` 目录

### Bundled Recipe 的升级规则

内置 bundled recipe 现在采用“`digest 判定，显式升级`”模型：

- 首次启动时，如果 workspace 缺失，会自动 seed
- 如果 bundled source 更新了，但用户没有改本地副本，UI 会显示 `Update available`
- 如果用户改过本地副本，不会被静默覆盖
- 只有用户显式点击升级，workspace copy 才会被替换

状态语义：

- `upToDate`
- `updateAvailable`
- `localModified`
- `conflictedUpdate`

这里 `version` 只用于展示；真正判断是否有升级，始终看 source `digest`。

### 来源、信任与批准

workspace recipe 会记录来源：

- `bundled`
- `localImport`
- `remoteUrl`

这会影响执行前的信任和批准规则：

- `bundled`
  普通变更默认可执行，高风险动作需要批准
- `localImport`
  中风险和高风险 recipe 首次执行前需要批准
- `remoteUrl`
  任何会修改环境的 recipe 首次执行前都需要批准

批准是按 `workspace recipe + 当前 digest` 记忆的：

- 同一个 digest 只需批准一次
- 只要 recipe 被编辑、重新导入或升级，digest 变化，批准自动失效

## 2. 推荐的作者目录结构

新增一个可维护的 Recipe，推荐放在独立目录里，而不是直接写进 `src-tauri/recipes.json`。

当前仓库采用的结构是：

```text
examples/recipe-library/
  dedicated-agent/
    recipe.json
  agent-persona-pack/
    recipe.json
    assets/
      personas/
        coach.md
        researcher.md
  channel-persona-pack/
    recipe.json
    assets/
      personas/
        incident.md
        support.md
```

规则：
- 每个 Recipe 一个目录
- 目录里必须有 `recipe.json`
- 如需预设 markdown 文本，放到 `assets/`
- import 时只扫描 library 根目录下的一级子目录

## 3. 顶层文档形状

对于 library 里的 `recipe.json`，推荐写成单个 recipe 对象。

当前加载器支持三种形状：

```json
{ "...": "single recipe object" }
```

```json
[
  { "...": "recipe 1" },
  { "...": "recipe 2" }
]
```

```json
{
  "recipes": [
    { "...": "recipe 1" },
    { "...": "recipe 2" }
  ]
}
```

但有一个关键区别：
- `Load` 文件或 URL 时，可以接受三种形状
- `Import` 外部 recipe library 时，`recipe.json` 必须是单个对象

因此，写新的 library recipe 时，直接使用单对象。

## 4. 一个完整 Recipe 的推荐结构

当前推荐写法：

```json
{
  "id": "dedicated-agent",
  "name": "Dedicated Agent",
  "description": "Create an agent and set its identity and persona",
  "version": "1.0.0",
  "tags": ["agent", "identity", "persona"],
  "difficulty": "easy",
  "presentation": {
    "resultSummary": "Created dedicated agent {{name}} ({{agent_id}})"
  },
  "params": [],
  "steps": [],
  "bundle": {},
  "executionSpecTemplate": {},
  "clawpalImport": {}
}
```

字段职责：
- `id / name / description / version / tags / difficulty`
  Recipe 元信息
- `presentation`
  面向用户的结果文案
- `params`
  Configure 阶段的参数表单
- `steps`
  面向用户的步骤文案
- `bundle`
  声明 capability、resource claim、execution kind 的白名单
- `executionSpecTemplate`
  真正要编译成什么 `ExecutionSpec`
- `clawpalImport`
  仅用于 library import 阶段的扩展元数据，不会保留在最终 workspace recipe 里

## 5. 参数字段怎么写

`params` 是数组，每项形状如下：

```json
{
  "id": "agent_id",
  "label": "Agent ID",
  "type": "string",
  "required": true,
  "placeholder": "e.g. ops-bot",
  "pattern": "^[a-z0-9-]+$",
  "minLength": 3,
  "maxLength": 32,
  "defaultValue": "main",
  "dependsOn": "advanced",
  "options": [
    { "value": "coach", "label": "Coach" }
  ]
}
```

当前前端支持的 `type`：
- `string`
- `number`
- `boolean`
- `textarea`
- `discord_guild`
- `discord_channel`
- `model_profile`
- `agent`

UI 规则：
- `options` 非空时，优先渲染为下拉
- `discord_guild` 从当前环境加载 guild 列表
- `discord_channel` 从当前环境加载 channel 列表
- `agent` 从当前环境加载 agent 列表
- `model_profile` 从当前环境加载可用 model profiles
- `dependsOn` 当前仍是简单门控，不要依赖复杂表达式

实用建议：
- 长文本输入用 `textarea`
- 固定预设优先用 `options`
- `model_profile` 如果希望默认跟随环境，可用 `__default__`

## 6. `steps` 和 `executionSpecTemplate.actions` 必须一一对应

`steps` 是给用户看的，`executionSpecTemplate.actions` 是给编译器和 runner 看的。

当前校验要求：
- `steps.len()` 必须等于 `executionSpecTemplate.actions.len()`
- 每一步的 `action` 应与对应 action 的 `kind` 保持一致

也就是说，`steps` 不是装饰层，它是用户理解“这次会做什么”的主入口。

## 7. 当前支持的 action surface

当前 Recipe DSL 的 action 分两层：

- 推荐层：高层业务动作，优先给大多数 recipe 作者使用
- 高级层：CLI 原语动作，按 OpenClaw CLI 子命令 1:1 暴露

此外还有：
- 文档底座动作
- 环境编排动作
- legacy/escape hatch

### 7.1 推荐的业务动作

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

推荐：
- 新的业务 recipe 优先使用业务动作
- `set_agent_identity` 优于旧的 `setup_identity`
- `bind_agent` / `unbind_agent` 优于旧的 `bind_channel` / `unbind_channel`

### 7.2 文档动作

- `upsert_markdown_document`
- `delete_markdown_document`

这是高级/底座动作，适合：
- 写 agent 默认 markdown 文档
- 直接控制 section upsert 或 whole-file replace

### 7.3 环境动作

- `ensure_model_profile`
- `delete_model_profile`
- `ensure_provider_auth`
- `delete_provider_auth`

这组动作负责：
- 确保目标环境存在可用 profile
- 必要时同步 profile 依赖的 auth/secret
- 清理不再需要的 auth/profile

### 7.4 CLI 原语动作

对于需要直接复用 OpenClaw CLI 的高级 recipe，可以使用 CLI 原语动作。

当前 catalog 覆盖了这些命令组：
- `agents`
- `config`
- `models`
- `channels`
- `secrets`

例子：
- `list_agents` -> `openclaw agents list`
- `list_agent_bindings` -> `openclaw agents bindings`
- `show_config_file` -> `openclaw config file`
- `get_config_value` / `set_config_value` / `unset_config_value`
- `models_status` / `list_models` / `set_default_model`
- `list_channels` / `channels_status` / `inspect_channel_capabilities`
- `reload_secrets` / `audit_secrets` / `apply_secrets_plan`

完整清单见：[recipe-cli-action-catalog.md](./recipe-cli-action-catalog.md)

注意：
- 文档里出现并不等于 runner 一定支持执行
- interactive 或携带 secret payload 的 CLI 子命令，只会记录在 catalog 里，不建议写进 recipe

## 7.6 Review 阶段现在会严格阻断什么

当前 `Cook -> Review` 会把下面这些情况当成阻断项，而不是“执行后再失败”：

- 当前 recipe 需要批准，但还没批准
- auth 预检返回 `error`
- destructive action 默认删除仍被引用的资源

因此作者在设计 recipe 时，应优先做到：

- 结果语义清晰
- claim 和 capability 可稳定推导
- destructive 行为显式声明 `force` / `rebind` 之类的意图参数

### 7.5 兼容 / escape hatch

- `config_patch`
- `setup_identity`
- `bind_channel`
- `unbind_channel`

保留用于兼容旧 recipe 或极少数低层配置改写，但不建议作为 bundled recipe 的主路径。

## 8. 各类 action 的常见输入

### `create_agent`

```json
{
  "kind": "create_agent",
  "args": {
    "agentId": "{{agent_id}}",
    "modelProfileId": "{{model}}"
  }
}
```

说明：
- 旧的 `independent` 字段仍可被兼容读取，但不再推荐使用
- workspace 由 OpenClaw 默认策略决定；runner 不再把 `agentId` 直接当成 workspace 路径

### `set_agent_identity`

```json
{
  "kind": "set_agent_identity",
  "args": {
    "agentId": "{{agent_id}}",
    "name": "{{name}}",
    "emoji": "{{emoji}}"
  }
}
```

### `set_agent_persona`

```json
{
  "kind": "set_agent_persona",
  "args": {
    "agentId": "{{agent_id}}",
    "persona": "{{presetMap:persona_preset}}"
  }
}
```

### `bind_agent`

```json
{
  "kind": "bind_agent",
  "args": {
    "agentId": "{{agent_id}}",
    "binding": "discord:{{channel_id}}"
  }
}
```

### `set_channel_persona`

```json
{
  "kind": "set_channel_persona",
  "args": {
    "channelType": "discord",
    "guildId": "{{guild_id}}",
    "peerId": "{{channel_id}}",
    "persona": "{{presetMap:persona_preset}}"
  }
}
```

### `upsert_markdown_document`

```json
"args": {
  "target": {
    "scope": "agent",
    "agentId": "{{agent_id}}",
    "path": "IDENTITY.md"
  },
  "mode": "replace",
  "content": "- Name: {{name}}\n\n## Persona\n{{persona}}\n"
}
```

支持的 `target.scope`：
- `agent`
- `home`
- `absolute`

支持的 `mode`：
- `replace`
- `upsertSection`

`upsertSection` 需要额外提供：
- `heading`
- 可选 `createIfMissing`

### `delete_markdown_document`

```json
"args": {
  "target": {
    "scope": "agent",
    "agentId": "{{agent_id}}",
    "path": "PLAYBOOK.md"
  },
  "missingOk": true
}
```

### `ensure_model_profile`

```json
{
  "kind": "ensure_model_profile",
  "args": {
    "profileId": "{{model}}"
  }
}
```

### `ensure_provider_auth`

```json
{
  "kind": "ensure_provider_auth",
  "args": {
    "provider": "openrouter",
    "authRef": "openrouter:default"
  }
}
```

### destructive 动作

以下动作默认会做引用检查，仍被引用时会失败：
- `delete_agent`
- `delete_model_profile`
- `delete_provider_auth`

显式 override：
- `delete_agent.force`
- `delete_agent.rebindChannelsTo`
- `delete_provider_auth.force`
- `delete_model_profile.deleteAuthRef`

### CLI 原语动作例子

```json
{
  "kind": "get_config_value",
  "args": {
    "path": "gateway.port"
  }
}
```

```json
{
  "kind": "models_status",
  "args": {
    "probe": true,
    "probeProvider": "openai"
  }
}
```

## 9. `bundle` 写什么

`bundle` 的作用是声明：
- 允许使用哪些 capability
- 允许触碰哪些 resource kind
- 支持哪些 execution kind

例如：

```json
"bundle": {
  "apiVersion": "strategy.platform/v1",
  "kind": "StrategyBundle",
  "metadata": {
    "name": "dedicated-agent",
    "version": "1.0.0",
    "description": "Create a dedicated agent"
  },
  "compatibility": {},
  "inputs": [],
  "capabilities": {
    "allowed": ["agent.manage", "agent.identity.write", "model.manage", "secret.sync"]
  },
  "resources": {
    "supportedKinds": ["agent", "modelProfile"]
  },
  "execution": {
    "supportedKinds": ["job"]
  },
  "runner": {},
  "outputs": [{ "kind": "recipe-summary", "recipeId": "dedicated-agent" }]
}
```

当前常见 capability：
- `agent.manage`
- `agent.identity.write`
- `binding.manage`
- `config.write`
- `document.write`
- `document.delete`
- `model.manage`
- `auth.manage`
- `secret.sync`

当前常见 resource claim kind：
- `agent`
- `channel`
- `file`
- `document`
- `modelProfile`
- `authProfile`

## 10. `executionSpecTemplate` 写什么

它定义编译后真正的 `ExecutionSpec`，通常至少要包含：

```json
"executionSpecTemplate": {
  "apiVersion": "strategy.platform/v1",
  "kind": "ExecutionSpec",
  "metadata": {
    "name": "dedicated-agent"
  },
  "source": {},
  "target": {},
  "execution": {
    "kind": "job"
  },
  "capabilities": {
    "usedCapabilities": ["model.manage", "secret.sync", "agent.manage", "agent.identity.write"]
  },
  "resources": {
    "claims": [
      { "kind": "modelProfile", "id": "{{model}}" },
      { "kind": "agent", "id": "{{agent_id}}" }
    ]
  },
  "secrets": {
    "bindings": []
  },
  "desiredState": {
    "actionCount": 4
  },
  "actions": [
    {
      "kind": "ensure_model_profile",
      "name": "Prepare model access",
      "args": {
        "profileId": "{{model}}"
      }
    },
    {
      "kind": "create_agent",
      "name": "Create dedicated agent",
      "args": {
        "agentId": "{{agent_id}}",
        "modelProfileId": "{{model}}"
      }
    },
    {
      "kind": "set_agent_identity",
      "name": "Set agent identity",
      "args": {
        "agentId": "{{agent_id}}",
        "name": "{{name}}",
        "emoji": "{{emoji}}"
      }
    },
    {
      "kind": "set_agent_persona",
      "name": "Set agent persona",
      "args": {
        "agentId": "{{agent_id}}",
        "persona": "{{persona}}"
      }
    }
  ],
  "outputs": [{ "kind": "recipe-summary", "recipeId": "dedicated-agent" }]
}
```

当前 `execution.kind` 支持：
- `job`
- `service`
- `schedule`
- `attachment`

对大多数业务 recipe：
- 一次性业务动作优先用 `job`
- 配置附着类动作可用 `attachment`

## 11. 模板变量

当前支持两类最常用模板。

### 11.1 参数替换

```json
"agentId": "{{agent_id}}"
```

### 11.2 preset map 替换

```json
"persona": "{{presetMap:persona_preset}}"
```

这类变量只在 import 后的 workspace recipe 里使用编译好的 map，不会在运行时继续去读外部 `assets/`。

## 12. `clawpalImport` 和 `assets/`

如果 recipe 需要把外部 markdown 资产编译进最终 recipe，可以使用：

```json
"clawpalImport": {
  "presetParams": {
    "persona_preset": [
      { "value": "coach", "label": "Coach", "asset": "assets/personas/coach.md" },
      { "value": "researcher", "label": "Researcher", "asset": "assets/personas/researcher.md" }
    ]
  }
}
```

import 阶段会做三件事：
- 校验 `asset` 是否存在
- 为目标 param 注入 `options`
- 把 `{{presetMap:param_id}}` 编译成内嵌文本映射

最终写入 workspace 的 recipe：
- 不再保留 `clawpalImport`
- 不再依赖原始 `assets/` 目录
- 会带 `clawpalPresetMaps`

## 13. `presentation` 怎么用

如果希望 `Done`、`Recent Recipe Runs`、`Orchestrator` 显示更业务化的结果，给 recipe 增加：

```json
"presentation": {
  "resultSummary": "Updated persona for agent {{agent_id}}"
}
```

原则：
- 写给非技术用户看
- 描述“得到什么结果”，不要描述执行细节
- 没写时会退回到通用 summary

## 14. OpenClaw-first 原则

作者在写 Recipe 时要默认遵循：

- 能用业务动作表达的，不要退回 `config_patch`
- 能用 OpenClaw 原语表达的，让 runner 优先走 OpenClaw
- 文档动作只在 OpenClaw 还没有对应原语时作为底座

例如：
- `set_channel_persona` 优于手写 `config_patch`
- `ensure_model_profile` 优于假定目标环境已经有 profile
- `upsert_markdown_document` 适合写 agent 默认 markdown 文档

更详细的边界见：[recipe-runner-boundaries.md](./recipe-runner-boundaries.md)

## 15. 最小验证流程

新增或修改 recipe 后，至少做这几步：

1. 校验 Rust 侧 recipe 测试

```bash
cargo test recipe_ --lib --manifest-path src-tauri/Cargo.toml
```

2. 校验前端类型和关键 UI

```bash
bun run typecheck
```

3. 如改了导入规则或预置 recipe，验证 import/seed 结果

```bash
cargo test import_recipe_library_accepts_repo_example_library --manifest-path src-tauri/Cargo.toml
```

4. 如改了业务闭环，优先补 Docker OpenClaw e2e

## 16. 常见坑

- `steps` 和 `actions` 数量不一致会直接校验失败
- `Import` library 时，`recipe.json` 不能是数组
- `upsert_markdown_document` 的 `upsertSection` 模式必须带 `heading`
- `target.scope=agent` 时必须带 `agentId`
- 相对路径里不允许 `..`
- destructive action 默认会被引用检查挡住
- recipe 不能内嵌明文 secret；环境动作只能引用 ClawPal 已能解析到的 secret/auth

如果你需要理解 runner 负责什么、不负责什么，再看：[recipe-runner-boundaries.md](./recipe-runner-boundaries.md)
