# Recipe 渠道无关化方案调研

## 日期
2026-02-20

## 现状

两个内置 recipe（`dedicated-channel-agent` 和 `discord-channel-persona`）都是硬编码为 Discord 专用。`config_patch` 模板包含 Discord 特定的 JSON 路径，如：

```
channels.discord.guilds.{{guild_id}}.channels.{{channel_id}}
```

## 各渠道配置结构

| 平台 | 路径结构 |
|------|----------|
| Discord | `channels.discord.guilds.{guildId}.channels.{channelId}` |
| Telegram | `channels.telegram.accounts.{accountId}` |
| Slack | `channels.slack.workspace.channels.{channelId}` |
| Mattermost | `channels.mattermost.workspace.channels.{channelId}` |

每个平台的嵌套层级和结构不同：
- Discord：guilds → channels（2层）
- Telegram：accounts（1层）
- Slack/Mattermost：workspace → channels（2层但结构不同）

## Recipe 执行管线

```
JSON recipe → ParamForm 收集用户输入 → 模板替换 ({{param}}) → 步骤执行 → deep merge 到配置
```

步骤类型（actions）：
- `create_agent` — 创建 agent
- `setup_identity` — 设置 identity
- `bind_channel` — 绑定频道
- `config_patch` — 深度合并配置补丁

## 核心难点

`config_patch` 模板中硬编码了 Discord 的 JSON 路径。要让 recipe 渠道无关，需要解决不同平台路径结构差异的问题。

## 建议方案

### 1. 新增 `platform` 参数类型

下拉菜单让用户选择目标平台（Discord / Telegram / Slack / Mattermost）。

### 2. 条件式 `config_patch`

Recipe 按平台定义不同的 patch 块，或引入模板助手如 `{{channel_path}}`，根据所选平台自动解析为正确的 JSON 路径。

示例：
```json
{
  "config_patch": {
    "{{channel_path}}": {
      "agent": "{{agent_name}}",
      "enabled": true
    }
  }
}
```

其中 `{{channel_path}}` 由引擎根据平台 + 用户选择的频道自动展开为：
- Discord: `channels.discord.guilds.xxx.channels.yyy`
- Telegram: `channels.telegram.accounts.xxx`
- Slack: `channels.slack.workspace.channels.xxx`

### 3. 抽象频道选择器

新增 `channel_selector` 参数类型，根据所选平台自动展示对应的选择器：
- Discord → 服务器 + 频道
- Telegram → 账号
- Slack/Mattermost → 工作区 + 频道

## 实现影响

- `src-tauri/recipes.json` — recipe 定义需要调整格式
- `src/pages/Cook.tsx` — ParamForm 需要支持新参数类型
- `src-tauri/src/commands.rs` — recipe 执行引擎需要支持路径解析
- Recipe 仍然是 JSON 配置文件，不需要硬编码平台细节

## 状态

调研完成，待实施。
