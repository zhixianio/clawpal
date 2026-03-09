import type {
  RescueDocHypothesis,
  RescuePrimaryDiagnosisResult,
  RescuePrimaryIssue,
  RescuePrimarySectionItem,
  RescuePrimarySectionResult,
  RescuePrimarySummary,
} from "@/lib/types";

const SECTION_ZH: Record<string, string> = {
  gateway: "网关",
  models: "模型",
  tools: "工具",
  agents: "Agent",
  channels: "频道",
  recovery: "恢复",
};

const EXACT_ZH: Record<string, string> = {
  Gateway: "网关",
  Models: "模型",
  Tools: "工具",
  Agents: "Agent",
  Channels: "频道",
  Recovery: "恢复",
  "Configuration needs attention": "配置需要处理",
  "Repair the OpenClaw configuration before the next check": "请先修复 OpenClaw 配置，再进行下一次检查",
  "Primary recovery checks look healthy": "Primary 恢复检查结果健康",
  "No recovery checks are available yet": "尚无可用恢复检查",
  "Configure and activate Rescue Bot before running recovery": "请先配置并启用恢复助手，再运行恢复检查",
  "Repair the blocking config error": "修复阻塞性的配置错误",
  "Gateway port": "网关端口",
  "Provider configuration": "Provider 配置",
  "Primary model binding": "Primary 模型绑定",
  "Tooling surface": "工具配置面",
  "Agent definitions": "Agent 定义",
  "Configured channel surfaces": "已配置频道面",
  "Configuration unavailable": "配置不可用",
  "OpenClaw doctor report": "OpenClaw Doctor 报告",
  "Primary doctor report": "Primary Doctor 报告",
  "Primary gateway status": "Primary 网关状态",
  "Rescue gateway status": "Rescue 网关状态",
  "Rescue profile configured": "Rescue Profile 已配置",
  "OpenClaw doctor command failed": "OpenClaw Doctor 命令执行失败",
  "Primary doctor command failed": "Primary Doctor 命令执行失败",
  "Review doctor output and gateway logs for details": "请查看 Doctor 输出和网关日志获取详细信息",
  "Primary configuration could not be read": "无法读取 Primary 配置",
  "Repair openclaw.json parsing errors and re-run diagnosis": "请修复 openclaw.json 解析错误后重新诊断",
  "Repair openclaw.json parsing errors and re-run the primary recovery check": "请修复 openclaw.json 解析错误后重新运行 Primary 恢复检查",
  "Primary gateway is not healthy": "Primary 网关未处于健康状态",
  "Rescue gateway is not healthy": "Rescue 网关未处于健康状态",
  "Restart the primary gateway and inspect logs if it stays unhealthy": "重启 Primary 网关；若仍不健康，请查看日志。",
  "Restart primary gateway and inspect gateway logs if it stays unhealthy": "重启 Primary 网关；若仍不健康，请查看网关日志。",
  "Restart primary gateway": "重启 Primary 网关",
  "Missing agent defaults": "缺少 Agent 默认配置",
  "Initialize agents.defaults.model": "初始化 agents.defaults.model",
  "Review helper permissions": "检查辅助权限",
  "Allowlist blocks rescue helper access": "Allowlist 阻止了恢复助手访问",
  "Expand tools.allow and sessions visibility": "扩大 tools.allow 和 sessions 可见性",
  "Review tool allowlist": "检查工具 Allowlist",
  "Narrow tool scope": "收窄工具权限范围",
  "No model providers are configured": "未配置任何模型 Provider",
  "No default model binding is configured": "未配置默认模型绑定",
  "Tools config exists but has no explicit controls": "工具配置已存在，但没有显式控制项",
  "No explicit tools configuration found": "未找到显式工具配置",
  "No explicit agents.list entries were found": "未找到显式的 agents.list 条目",
  "No channels are configured": "尚未配置任何频道",
  "Configuration could not be read for this target": "无法读取该目标的配置",
  "Gateway is not running": "网关未运行",
  "gateway not healthy": "网关不健康",
  "Gateway is healthy": "网关健康",
  "Channels are inactive": "频道未启用",
  "Agent defaults are missing": "缺少 Agent 默认配置",
  "The primary profile has no agents.defaults.model binding.": "Primary 配置缺少 agents.defaults.model 绑定。",
  "Set agents.defaults.model to a valid provider/model pair.": "将 agents.defaults.model 设置为有效的 provider/model 组合。",
  "Re-run the primary check after saving the config.": "保存配置后重新运行 Primary 检查。",
  "Guidance matches OpenClaw 2026.3.x.": "诊断建议适配 OpenClaw 2026.3.x。",
  "Repair the syntax error in ~/.openclaw/openclaw.json": "修复 ~/.openclaw/openclaw.json 中的语法错误",
};

function isChineseUi(language?: string | null): boolean {
  return language?.toLowerCase().startsWith("zh") ?? false;
}

function sectionNameZh(raw: string): string {
  const normalized = raw.trim().toLowerCase();
  return SECTION_ZH[normalized] ?? raw;
}

function localizeCommaList(value: string): string {
  return value
    .split(/\s*,\s*/)
    .map((part) => sectionNameZh(part))
    .join("、");
}

function localizeKeyValueList(text: string): string | null {
  if (!/^[a-z_]+=[^,]+(?:,\s*[a-z_]+=[^,]+)*$/.test(text.trim())) {
    return null;
  }

  const keyZh: Record<string, string> = {
    running: "运行中",
    healthy: "健康",
    port: "端口",
    state: "状态",
    rpc: "RPC",
    port_status: "端口状态",
    profile: "profile",
  };

  return text
    .split(/\s*,\s*/)
    .map((part) => {
      const [key, ...rest] = part.split("=");
      const value = rest.join("=");
      return `${keyZh[key] ?? key}=${value}`;
    })
    .join("，");
}

type Replacer = {
  pattern: RegExp;
  replace: (...groups: string[]) => string;
};

const REGEX_ZH: Replacer[] = [
  {
    pattern: /^([A-Za-z]+) needs attention first$/,
    replace: (section) => `${sectionNameZh(section)}需要优先处理`,
  },
  {
    pattern: /^([A-Za-z]+) has recommended improvements$/,
    replace: (section) => `${sectionNameZh(section)}有建议优化项`,
  },
  {
    pattern: /^Apply (\d+) fix(?:es)? and re-run recovery$/,
    replace: (count) => `应用 ${count} 个修复后重新检查`,
  },
  {
    pattern: /^Apply (\d+) optimization(?:s)? and re-run recovery$/,
    replace: (count) => `应用 ${count} 个优化后重新检查`,
  },
  {
    pattern: /^Apply (\d+) optimization(?:s)? to stabilize the target$/,
    replace: (count) => `应用 ${count} 个优化来稳定目标配置`,
  },
  {
    pattern: /^Apply (\d+) optimization$/,
    replace: (count) => `应用 ${count} 个优化`,
  },
  {
    pattern: /^Review (.+) findings and fix them manually$/,
    replace: (section) => `请手动检查并修复${sectionNameZh(section)}中的问题`,
  },
  {
    pattern: /^Review (.+) recommendations before the next check$/,
    replace: (section) => `请在下次检查前先处理${sectionNameZh(section)}的建议项`,
  },
  {
    pattern: /^Keep monitoring (.+) and re-run checks after changes$/,
    replace: (section) => `继续关注${sectionNameZh(section)}，变更后重新检查`,
  },
  {
    pattern: /^([A-Za-z]+) has (\d+) blocking finding(?:\(s\)|s)?$/,
    replace: (section, count) => `${sectionNameZh(section)}有 ${count} 个阻塞性问题`,
  },
  {
    pattern: /^([A-Za-z]+) has (\d+) recommended change(?:\(s\)|s)?$/,
    replace: (section, count) => `${sectionNameZh(section)}有 ${count} 个建议调整`,
  },
  {
    pattern: /^([A-Za-z]+) checks look healthy$/,
    replace: (section) => `${sectionNameZh(section)}检查正常`,
  },
  {
    pattern: /^([A-Za-z]+) is not configured yet$/,
    replace: (section) => `${sectionNameZh(section)}尚未配置`,
  },
  {
    pattern: /^Configured primary gateway port: (\d+)$/,
    replace: (port) => `Primary 网关端口已配置为 ${port}`,
  },
  {
    pattern: /^Configured providers: (.+)$/,
    replace: (providers) => `已配置 Provider：${localizeCommaList(providers)}`,
  },
  {
    pattern: /^Primary model resolves to (.+)$/,
    replace: (model) => `Primary 模型解析为 ${model}`,
  },
  {
    pattern: /^Configured tool controls: (.+)$/,
    replace: (controls) => `已配置工具控制项：${localizeCommaList(controls)}`,
  },
  {
    pattern: /^Configured agents: (\d+)$/,
    replace: (count) => `已配置 ${count} 个 Agent`,
  },
  {
    pattern: /^Configured channel nodes: (\d+) \((.+)\)$/,
    replace: (count, kinds) => `已配置 ${count} 个频道节点（${localizeCommaList(kinds)}）`,
  },
  {
    pattern: /^Rescue profile "([^"]+)" is not configured$/,
    replace: (profile) => `Rescue Profile「${profile}」尚未配置`,
  },
  {
    pattern: /^Applied (\d+) fix(?:es)?\.$/,
    replace: (count) => `已应用 ${count} 个修复。`,
  },
  {
    pattern: /^Applied (\d+) optimization(?:s)?\.$/,
    replace: (count) => `已应用 ${count} 个优化。`,
  },
];

export function localizeDoctorReportText(text: string, language?: string | null): string {
  if (!text || !isChineseUi(language)) {
    return text;
  }

  const exact = EXACT_ZH[text];
  if (exact) {
    return exact;
  }

  for (const entry of REGEX_ZH) {
    const match = text.match(entry.pattern);
    if (!match) {
      continue;
    }
    return entry.replace(...match.slice(1));
  }

  return localizeKeyValueList(text) ?? text;
}

function localizeGuidanceHypotheses(
  items: RescueDocHypothesis[] | undefined,
  language?: string | null,
): RescueDocHypothesis[] | undefined {
  return items?.map((item) => ({
    ...item,
    title: localizeDoctorReportText(item.title, language),
    reason: localizeDoctorReportText(item.reason, language),
  }));
}

function localizeGuidanceSteps(
  steps: string[] | undefined,
  language?: string | null,
): string[] | undefined {
  return steps?.map((step) => localizeDoctorReportText(step, language));
}

function localizeSummary(
  summary: RescuePrimarySummary,
  language?: string | null,
): RescuePrimarySummary {
  return {
    ...summary,
    headline: localizeDoctorReportText(summary.headline, language),
    recommendedAction: localizeDoctorReportText(summary.recommendedAction, language),
    rootCauseHypotheses: localizeGuidanceHypotheses(summary.rootCauseHypotheses, language),
    fixSteps: localizeGuidanceSteps(summary.fixSteps, language),
    versionAwareness: summary.versionAwareness
      ? localizeDoctorReportText(summary.versionAwareness, language)
      : summary.versionAwareness,
  };
}

function localizeSectionItem(
  item: RescuePrimarySectionItem,
  language?: string | null,
): RescuePrimarySectionItem {
  return {
    ...item,
    label: localizeDoctorReportText(item.label, language),
    detail: localizeDoctorReportText(item.detail, language),
  };
}

function localizeSection(
  section: RescuePrimarySectionResult,
  language?: string | null,
): RescuePrimarySectionResult {
  return {
    ...section,
    title: SECTION_ZH[section.key] ?? localizeDoctorReportText(section.title, language),
    summary: localizeDoctorReportText(section.summary, language),
    items: section.items.map((item) => localizeSectionItem(item, language)),
    rootCauseHypotheses: localizeGuidanceHypotheses(section.rootCauseHypotheses, language),
    fixSteps: localizeGuidanceSteps(section.fixSteps, language),
    versionAwareness: section.versionAwareness
      ? localizeDoctorReportText(section.versionAwareness, language)
      : section.versionAwareness,
  };
}

function localizeIssue(
  issue: RescuePrimaryIssue,
  language?: string | null,
): RescuePrimaryIssue {
  return {
    ...issue,
    message: localizeDoctorReportText(issue.message, language),
    fixHint: issue.fixHint
      ? localizeDoctorReportText(issue.fixHint, language)
      : issue.fixHint,
  };
}

export function localizeRescuePrimaryDiagnosis(
  diagnosis: RescuePrimaryDiagnosisResult,
  language?: string | null,
): RescuePrimaryDiagnosisResult {
  if (!isChineseUi(language)) {
    return diagnosis;
  }

  return {
    ...diagnosis,
    summary: localizeSummary(diagnosis.summary, language),
    sections: diagnosis.sections.map((section) => localizeSection(section, language)),
    issues: diagnosis.issues.map((issue) => localizeIssue(issue, language)),
  };
}
