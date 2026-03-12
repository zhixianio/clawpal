# 如何编写一个 ClawPal Recipe

这份文档描述的是当前仓库里真实可用的 Recipe 写法，不是早期的设计草案。

目标读者：
- 需要新增一个内置 Recipe 的开发者
- 需要维护 `examples/recipe-library/` 外部 Recipe 库的人
- 需要理解 `bundle + executionSpecTemplate + steps` 三层结构的人

## 1. 先理解 Recipe 在当前产品里的位置

当前 ClawPal 的 Recipe 有两种进入方式：

1. 作为预置 Recipe 随 App 打包
2. 作为外部 Recipe library 在运行时导入

无论入口是哪一种，最终都会落到 workspace recipe：

`~/.clawpal/recipes/workspace/<slug>.recipe.json`

也就是说，Recipe 的运行时载体始终是一个自包含的单文件 JSON。

## 2. 推荐的作者目录结构

如果你要新增一个可维护的 Recipe，推荐放在独立目录里，而不是直接手写到 `src-tauri/recipes.json`。

当前仓库采用的目录结构是：

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
- 如果要引用预设文案或 markdown 资源，用 `assets/` 子目录

## 3. 顶层文档形状

对于 library 目录里的 `recipe.json`，推荐写成单个对象。

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
- `load recipes from file / URL` 可以接受数组和 `{ recipes: [] }`
- `import recipe library` 的 `recipe.json` 必须是单个 recipe 对象

所以新写一个 library recipe 时，直接用单对象。

## 4. 一个完整 Recipe 的推荐结构

当前推荐写法是：

```json
{
  "id": "dedicated-agent",
  "name": "Dedicated Agent",
  "description": "Create an independent agent and set its identity and persona",
  "version": "1.0.0",
  "tags": ["agent", "identity", "persona"],
  "difficulty": "easy",
  "presentation": {
    "resultSummary": "Created dedicated agent {{name}} ({{agent_id}})"
  },
  "params": [],
  "steps": [],
  "bundle": {},
  "executionSpecTemplate": {}
}
```

这几个字段的职责要分清：

- `id / name / description / version / tags / difficulty`
  Recipe 元信息
- `presentation`
  面向用户的结果文案
- `params`
  Configure 阶段的参数表单
- `steps`
  面向用户的步骤文案和顺序
- `bundle`
  允许做什么，允许碰哪些资源
- `executionSpecTemplate`
  真正执行时会被编译成什么

## 5. 参数字段怎么写

`params` 是一个数组，每项形状如下：

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
  "dependsOn": "independent",
  "options": [
    { "value": "coach", "label": "Coach" }
  ]
}
```

当前前端实际支持的 `type`：

- `string`
- `number`
- `boolean`
- `textarea`
- `discord_guild`
- `discord_channel`
- `model_profile`
- `agent`

UI 特殊行为：
- `options` 非空时，优先渲染成下拉选择
- `discord_guild` 会从当前实例里加载 guild 列表
- `discord_channel` 会从当前实例里加载 channel 列表
- `agent` 会从当前实例里加载 agent 列表
- `model_profile` 会从当前实例里加载可用 model profiles
- `dependsOn` 当前只支持布尔门控：只有依赖参数值等于 `"true"` 时才显示

实用建议：
- `model_profile` 参数的默认值通常写 `__default__`
- 需要用户自由填写长文本时用 `textarea`
- 需要作者控制选项集合时优先用 `options`

## 6. `steps` 和 `executionSpecTemplate.actions` 必须对齐

`steps` 是给用户看的，`executionSpecTemplate.actions` 是给执行器看的。

当前校验要求：
- `steps.len()` 必须等于 `executionSpecTemplate.actions.len()`

也就是说，`steps` 不是可有可无的装饰层，它必须和执行动作一一对应。

例如：

```json
"steps": [
  {
    "action": "create_agent",
    "label": "Create dedicated agent",
    "args": {
      "agentId": "{{agent_id}}",
      "modelProfileId": "{{model}}",
      "independent": true
    }
  },
  {
    "action": "setup_identity",
    "label": "Set agent identity",
    "args": {
      "agentId": "{{agent_id}}",
      "name": "{{name}}",
      "emoji": "{{emoji}}",
      "persona": "{{persona}}"
    }
  }
]
```

## 7. 当前真正支持的 action

当前 action materializer 明确支持这四种：

- `create_agent`
- `setup_identity`
- `bind_channel`
- `config_patch`

它们的实际语义是：

- `create_agent`
  通过 OpenClaw CLI 创建 agent
- `setup_identity`
  写 agent 的 `IDENTITY.md`
- `bind_channel`
  通过 OpenClaw CLI 改写 `bindings`
- `config_patch`
  通过 OpenClaw CLI 改写配置树

写 Recipe 时不要假设别的 action 已经可用。要先看执行器和 materializer 有没有实现。

## 8. 当前支持的 execution kind

当前 `execution.kind` 支持：

- `job`
- `service`
- `schedule`
- `attachment`

但对 Recipe 作者来说，可以先按下面理解：

- `job`
  一次性动作，最常见
- `attachment`
  更适合配置改写或附加状态
- `service` / `schedule`
  主要给结构化 systemd 计划使用

当前三条业务 recipe 主要用的是：
- `job`
- `attachment`

## 9. `bundle` 写什么

`bundle` 的作用是声明：
- 允许使用哪些 capability
- 允许声明哪些 resource claim
- 支持哪些 execution kind

最常见的写法是：

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
    "allowed": ["agent.manage", "agent.identity.write"]
  },
  "resources": {
    "supportedKinds": ["agent"]
  },
  "execution": {
    "supportedKinds": ["job"]
  },
  "runner": {},
  "outputs": [
    { "kind": "recipe-summary", "recipeId": "dedicated-agent" }
  ]
}
```

当前资源 claim kind 白名单是：

- `path`
- `file`
- `service`
- `channel`
- `agent`
- `identity`

## 10. `executionSpecTemplate` 写什么

`executionSpecTemplate` 是真正会被渲染参数的执行模板。

一个常见例子：

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
    "usedCapabilities": ["agent.manage", "agent.identity.write"]
  },
  "resources": {
    "claims": [
      { "kind": "agent", "id": "{{agent_id}}" }
    ]
  },
  "secrets": {
    "bindings": []
  },
  "desiredState": {
    "actionCount": 2
  },
  "actions": [
    {
      "kind": "create_agent",
      "name": "Create dedicated agent",
      "args": {
        "agentId": "{{agent_id}}",
        "modelProfileId": "{{model}}",
        "independent": true
      }
    },
    {
      "kind": "setup_identity",
      "name": "Set agent identity",
      "args": {
        "agentId": "{{agent_id}}",
        "name": "{{name}}",
        "emoji": "{{emoji}}",
        "persona": "{{persona}}"
      }
    }
  ],
  "outputs": [
    { "kind": "recipe-summary", "recipeId": "dedicated-agent" }
  ]
}
```

实用规则：
- `metadata.name` 通常用 recipe id
- `source` 和 `target` 可以先留空，运行时会补上下文
- `desiredState.actionCount` 应和 actions 数量一致
- `resources.claims` 要能说明这次会碰到什么对象

## 11. 模板变量怎么渲染

当前支持两类占位符：

### 普通参数

```text
{{agent_id}}
{{channel_id}}
{{name}}
```

它会用参数值直接替换。

### 预设映射

```text
{{presetMap:persona_preset}}
```

它会根据当前参数值，从 `clawpalPresetMaps.persona_preset` 里取对应内容。

这个机制通常用于：
- persona preset
- system prompt preset
- 一组较长的 markdown 文案

注意：
- 占位符不仅能出现在值里，也能出现在对象 key 里
- `config_patch` 常会用这一点渲染 `guild_id` / `channel_id` 这种动态路径

## 12. 如何写预设资源型 Recipe

如果一个参数需要从 `assets/*.md` 这种资源文件里选预设，不建议手写 `clawpalPresetMaps`。

推荐写法是 `clawpalImport`：

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

导入器会做三件事：
- 把这些 asset 文件读进来
- 自动给对应 param 注入 `options`
- 自动生成 `clawpalPresetMaps`

所以作者只需要写：
- `params` 里保留 `persona_preset`
- `actions` 里引用 `{{presetMap:persona_preset}}`

不需要把大段 markdown 直接内联到 `recipe.json`。

## 13. `presentation.resultSummary` 是给谁看的

这个字段会直接影响：
- `Done` 页的结果摘要
- `Recent Recipe Runs`
- 其他结果导向的 UI

例如：

```json
"presentation": {
  "resultSummary": "Updated persona for agent {{agent_id}}"
}
```

建议：
- 用业务结果句式
- 不要写技术实现细节
- 不要写 “via local runner” / “2 actions applied” 这种内部表达

好的例子：
- `Created dedicated agent {{name}} ({{agent_id}})`
- `Updated persona for agent {{agent_id}}`
- `Updated persona for channel {{channel_id}}`

## 14. 当前推荐的作者流程

### 方案 A：写一个预置 Recipe

1. 在 `examples/recipe-library/<your-recipe>/` 新建目录
2. 写 `recipe.json`
3. 如果需要 preset 资产，放进 `assets/`
4. 重启 app，让启动 seed 把它写进 workspace
5. 在 `Recipes` 页面直接验证

### 方案 B：写一个外部导入 Recipe

1. 在任意目录按相同结构组织 recipe library
2. 在 `Recipes` 页面用 `Import` 导入根目录
3. 导入后从 workspace 里打开 `Studio` 或 `Cook`

## 15. 最小验证命令

至少做这几类验证：

```bash
cargo test recipe_ --lib --manifest-path src-tauri/Cargo.toml
```

```bash
bun run typecheck
```

如果改了 `Cook / RecipePlanPreview / Orchestrator / ParamForm` 一类前端行为，再补对应前端测试。

## 16. 常见坑

### 1. `steps` 和 `actions` 数量不一致

这是当前最常见的 schema 错误之一。

### 2. 写了 UI 参数，但没在模板里用

这种参数不会产生实际效果，也容易误导用户。

### 3. `clawpalImport` 引用了不存在的 asset

导入时会直接失败。

### 4. 在 `bundle` 里没放 capability 或 resource kind

即使 `executionSpecTemplate` 写对了，也会被 bundle 校验挡住。

### 5. 把业务结果写成技术结果

`presentation.resultSummary` 应该描述“效果”，不是描述“执行细节”。

## 17. 建议从现有 3 个例子开始

当前最值得参考的例子在：

- [dedicated-agent/recipe.json](/Users/ChenYu/Documents/Github/clawpal/.worktrees/feat/recipe-import-library/examples/recipe-library/dedicated-agent/recipe.json)
- [agent-persona-pack/recipe.json](/Users/ChenYu/Documents/Github/clawpal/.worktrees/feat/recipe-import-library/examples/recipe-library/agent-persona-pack/recipe.json)
- [channel-persona-pack/recipe.json](/Users/ChenYu/Documents/Github/clawpal/.worktrees/feat/recipe-import-library/examples/recipe-library/channel-persona-pack/recipe.json)

它们分别覆盖了：
- 纯参数型 recipe
- 预设 persona 导入到 agent
- 预设 persona 导入到 channel

如果你要新增第四个 recipe，最稳的做法通常不是从零开始，而是从这三个里挑一个最接近的复制出来改。
