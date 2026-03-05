use crate::models::OpenClawPaths;
use crate::ssh::SshConnectionPool;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CACHE_SCHEMA_VERSION: u8 = 1;
const CACHE_TTL_SECONDS: u64 = 60 * 60 * 12;
const MAX_FETCH_PAGES: usize = 3;
const REMOTE_LLM_INDEX_URL: &str = "https://docs.openclaw.ai/llms.txt";
const REMOTE_LLM_FULL_URL: &str = "https://docs.openclaw.ai/llms-full.txt";
const REMOTE_SITEMAP_URL: &str = "https://docs.openclaw.ai/sitemap.xml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocResolveIssue {
    pub id: String,
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocResolveRequest {
    pub instance_scope: String,
    pub transport: String,
    pub openclaw_version: Option<String>,
    #[serde(default)]
    pub doctor_issues: Vec<DocResolveIssue>,
    pub config_content: String,
    pub error_log: String,
    pub gateway_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootCauseHypothesis {
    pub title: String,
    pub reason: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocCitation {
    pub url: String,
    pub section: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverMeta {
    pub cache_hit: bool,
    pub sources_checked: Vec<String>,
    pub rules_matched: Vec<String>,
    pub fetched_pages: usize,
    pub fallback_used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocGuidance {
    pub status: String,
    pub source_strategy: String,
    pub root_cause_hypotheses: Vec<RootCauseHypothesis>,
    pub fix_steps: Vec<String>,
    pub confidence: f32,
    pub citations: Vec<DocCitation>,
    pub version_awareness: String,
    pub resolver_meta: ResolverMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CachedEntry {
    fetched_at: u64,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ResolverCache {
    schema_version: u8,
    scope_version: HashMap<String, String>,
    entries: HashMap<String, CachedEntry>,
}

#[derive(Debug)]
struct RouteRule {
    id: &'static str,
    title: &'static str,
    reason: &'static str,
    keywords: &'static [&'static str],
    hints: &'static [&'static str],
    fix_steps: &'static [&'static str],
    weight: f32,
}

const ROUTE_RULES: &[RouteRule] = &[
    RouteRule {
        id: "provider_auth_failure",
        title: "Provider authentication mismatch",
        reason: "Signals suggest missing/expired api key or auth_ref/provider binding mismatch.",
        keywords: &[
            "invalid api key",
            "unauthorized",
            "auth_ref",
            "provider not found",
            "no auth profiles configured",
            "403",
            "401",
        ],
        hints: &[
            "auth-credential-semantics",
            "providers",
            "cli/auth",
            "automation/auth-monitoring",
        ],
        fix_steps: &[
            "Run `openclaw auth list` and verify the expected provider key exists.",
            "Run `clawpal doctor config-read providers` and confirm `auth_ref` points to a valid auth entry.",
            "If missing, add credential with `openclaw auth add` and retry `openclaw doctor --fix`.",
        ],
        weight: 0.95,
    },
    RouteRule {
        id: "group_policy_allowlist",
        title: "Group policy / allowlist restriction",
        reason: "Signals show groupPolicy, mention-gated groups, or allowFrom rejection.",
        keywords: &[
            "grouppolicy",
            "group policy",
            "mention-gated",
            "allowlist",
            "allowfrom",
            "pairing",
        ],
        hints: &[
            "channels/groups",
            "channels/group-messages",
            "channels/pairing",
            "channels/troubleshooting",
            "security",
        ],
        fix_steps: &[
            "Inspect channel/group policy: `clawpal doctor config-read channels`.",
            "Confirm target chat/group is in `allowFrom` and mention-gated policy allows this flow.",
            "Apply minimal policy fix then re-run `openclaw doctor --fix`.",
        ],
        weight: 0.92,
    },
    RouteRule {
        id: "gateway_connectivity",
        title: "Gateway connectivity/endpoint mismatch",
        reason: "Signals include gateway connection issues around ws://127.0.0.1:18789, token, or proxy headers.",
        keywords: &[
            "ws://127.0.0.1:18789",
            "gateway",
            "websocket",
            "proxy",
            "token",
            "connection refused",
        ],
        hints: &[
            "cli/gateway",
            "automation/troubleshooting",
            "channels/troubleshooting",
            "cli/health",
        ],
        fix_steps: &[
            "Run `openclaw gateway status` to verify gateway state and endpoint.",
            "Check configured gateway port via `clawpal doctor config-read gateway.port`.",
            "If endpoint/port mismatches, update config and restart gateway.",
        ],
        weight: 0.9,
    },
    RouteRule {
        id: "cron_heartbeat_conflict",
        title: "Cron and heartbeat conflict",
        reason: "Signals indicate cron/heartbeat overlap or duplicated automation triggers.",
        keywords: &[
            "cron",
            "heartbeat",
            "duplicate trigger",
            "scheduler",
            "watchdog",
        ],
        hints: &[
            "automation/cron-vs-heartbeat",
            "automation/cron-jobs",
            "automation/troubleshooting",
            "cli/cron",
        ],
        fix_steps: &[
            "List cron jobs and heartbeat-related automation settings.",
            "Disable overlapping schedules and keep a single trigger mechanism per workflow.",
            "Validate with a single controlled run before re-enabling periodic jobs.",
        ],
        weight: 0.86,
    },
    RouteRule {
        id: "tool_policy_denial",
        title: "Tool policy / sandbox denial",
        reason: "Signals include tools.elevated or sandbox/fs policy denials.",
        keywords: &[
            "tools.elevated",
            "sandbox",
            "denied by policy",
            "permission denied",
            "fs constraints",
            "approval",
        ],
        hints: &["cli/sandbox", "cli/approvals", "security", "cli/security"],
        fix_steps: &[
            "Review policy using `clawpal doctor config-read tools` and relevant security config.",
            "Allow only the required command/path scope; avoid broad wildcard grants.",
            "Retry the denied operation with explicit approval flow.",
        ],
        weight: 0.84,
    },
    RouteRule {
        id: "openclaw_path_missing",
        title: "OpenClaw binary path mismatch",
        reason: "Signals suggest openclaw command/path discovery failure.",
        keywords: &[
            "openclaw not found",
            "command not found",
            "binary not found",
            "failed to start",
            "path",
        ],
        hints: &["cli/doctor", "cli/configure", "install"],
        fix_steps: &[
            "Run `clawpal doctor probe-openclaw` to inspect binary path and PATH visibility.",
            "If missing path but binary exists, run `clawpal doctor fix-openclaw-path`.",
            "Probe again and verify `openclaw --version` resolves correctly.",
        ],
        weight: 0.82,
    },
    RouteRule {
        id: "provider_rate_limit",
        title: "Provider quota or rate limit",
        reason: "Signals indicate quota/rate-limit throttling from upstream provider.",
        keywords: &["quota exceeded", "rate limit", "429", "throttled", "forbidden"],
        hints: &["automation/auth-monitoring", "auth-credential-semantics", "providers"],
        fix_steps: &[
            "Confirm provider quota/limits and key scope in provider dashboard.",
            "Adjust retry/backoff policy and reduce burst traffic for the affected route.",
            "Switch to fallback provider/model profile if available.",
        ],
        weight: 0.8,
    },
    RouteRule {
        id: "channel_routing_mismatch",
        title: "Channel routing mismatch",
        reason: "Signals suggest channel mapping or route target mismatch.",
        keywords: &[
            "channel routing",
            "route",
            "channel override",
            "pairing code",
            "channel not found",
        ],
        hints: &[
            "channels/channel-routing",
            "channels/index",
            "channels/troubleshooting",
            "cli/channels",
        ],
        fix_steps: &[
            "Inspect effective routing with `clawpal doctor config-read channels`.",
            "Verify channel id and route mapping for the failing source.",
            "Apply targeted channel route correction and re-check delivery.",
        ],
        weight: 0.78,
    },
    RouteRule {
        id: "security_audit_block",
        title: "Security policy audit block",
        reason: "Signals indicate security audit or policy block conditions.",
        keywords: &[
            "security audit",
            "blocked",
            "policy violation",
            "unsafe",
            "denied",
        ],
        hints: &["security", "cli/security", "channels/group-messages"],
        fix_steps: &[
            "Review security policy section and identify the exact blocked capability.",
            "Use least-privilege exception scoped to the blocked workflow only.",
            "Re-run diagnosis and confirm no additional policy regressions.",
        ],
        weight: 0.77,
    },
    RouteRule {
        id: "gateway_proxy_headers",
        title: "Gateway proxy/header auth mismatch",
        reason: "Signals point to proxy and header forwarding mismatch around gateway auth.",
        keywords: &[
            "proxy headers",
            "x-forwarded",
            "invalid token",
            "gateway auth",
            "header",
        ],
        hints: &["cli/gateway", "automation/troubleshooting", "cli/health"],
        fix_steps: &[
            "Validate gateway auth header/token forwarding through proxy.",
            "Ensure proxy preserves websocket upgrade and required auth headers.",
            "Re-test with direct gateway access to isolate proxy-related failures.",
        ],
        weight: 0.76,
    },
];

#[derive(Debug, Clone)]
struct RuleMatch {
    rule: &'static RouteRule,
    score: f32,
}

#[derive(Debug, Clone)]
struct DocLink {
    title: String,
    url: String,
}

#[derive(Default)]
struct ResolveTelemetry {
    cache_hit: bool,
    sources_checked: Vec<String>,
    rules_matched: Vec<String>,
    fetched_pages: usize,
    fallback_used: bool,
}

pub async fn resolve_local_doc_guidance(
    request: &DocResolveRequest,
    paths: &OpenClawPaths,
) -> DocGuidance {
    resolve_doc_guidance_impl(None, None, request, paths).await
}

pub async fn resolve_remote_doc_guidance(
    pool: &SshConnectionPool,
    host_id: &str,
    request: &DocResolveRequest,
    paths: &OpenClawPaths,
) -> DocGuidance {
    resolve_doc_guidance_impl(Some(pool), Some(host_id), request, paths).await
}

async fn resolve_doc_guidance_impl(
    pool: Option<&SshConnectionPool>,
    host_id: Option<&str>,
    request: &DocResolveRequest,
    paths: &OpenClawPaths,
) -> DocGuidance {
    let mut cache = load_cache(paths);
    let mut telemetry = ResolveTelemetry::default();
    let scope_key = request.instance_scope.trim().to_string();
    invalidate_cache_if_version_changed(
        &mut cache,
        &scope_key,
        request.openclaw_version.as_deref(),
    );

    let client = Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok();

    let signature = build_signature_text(request);
    let rule_matches = match_route_rules(&signature);
    telemetry.rules_matched = rule_matches
        .iter()
        .map(|item| item.rule.id.to_string())
        .collect();

    let local_docs_root = match (pool, host_id) {
        (Some(p), Some(host)) => {
            telemetry
                .sources_checked
                .push("target-remote-docs".to_string());
            discover_remote_docs_root(p, host).await
        }
        _ => {
            telemetry
                .sources_checked
                .push("target-local-docs".to_string());
            discover_local_docs_root().and_then(|path| path.to_str().map(|s| s.to_string()))
        }
    };

    let mut index_links = Vec::new();
    if let Some(root) = &local_docs_root {
        let llms_text = match (pool, host_id) {
            (Some(p), Some(host)) => {
                let llms_path = format!("{}/llms.txt", root.trim_end_matches('/'));
                cached_remote_file(p, host, &llms_path, &scope_key, &mut cache, &mut telemetry)
                    .await
            }
            _ => {
                let llms_path = Path::new(root).join("llms.txt");
                cached_local_file(&llms_path, &scope_key, &mut cache, &mut telemetry)
            }
        };
        if let Some(content) = llms_text {
            let parsed = parse_llms_links(&content);
            if !parsed.is_empty() {
                telemetry
                    .sources_checked
                    .push("target-local-llms".to_string());
                index_links.extend(parsed);
            }
        }
    }

    if index_links.is_empty() {
        telemetry.fallback_used = true;
        telemetry
            .sources_checked
            .push("remote-llms-index".to_string());
        if let Some(client_ref) = client.as_ref() {
            if let Some(text) = cached_http_get(
                client_ref,
                REMOTE_LLM_INDEX_URL,
                "llms-index",
                &scope_key,
                &mut cache,
                &mut telemetry,
            )
            .await
            {
                index_links.extend(parse_llms_links(&text));
            }
        }
    }

    if index_links.is_empty() {
        telemetry.fallback_used = true;
        telemetry.sources_checked.push("remote-sitemap".to_string());
        if let Some(client_ref) = client.as_ref() {
            if let Some(text) = cached_http_get(
                client_ref,
                REMOTE_SITEMAP_URL,
                "sitemap",
                &scope_key,
                &mut cache,
                &mut telemetry,
            )
            .await
            {
                index_links.extend(parse_sitemap_links(&text));
            }
        }
    }

    if index_links.is_empty() {
        telemetry.fallback_used = true;
        telemetry
            .sources_checked
            .push("remote-llms-full".to_string());
        if let Some(client_ref) = client.as_ref() {
            if let Some(text) = cached_http_get(
                client_ref,
                REMOTE_LLM_FULL_URL,
                "llms-full",
                &scope_key,
                &mut cache,
                &mut telemetry,
            )
            .await
            {
                index_links.extend(parse_llms_full_sources(&text));
            }
        }
    }

    let keywords = top_keywords(&rule_matches);
    let mut ranked_urls = rank_urls_from_rules_and_index(&rule_matches, &index_links, &keywords);

    if ranked_urls.is_empty() && !rule_matches.is_empty() {
        telemetry.fallback_used = true;
        ranked_urls.extend(rule_matches.iter().flat_map(|item| {
            item.rule
                .hints
                .iter()
                .map(|hint| hint_to_url(hint))
                .collect::<Vec<_>>()
        }));
    }

    let mut citations = Vec::new();
    let mut snippets = Vec::new();
    let mut seen_urls = HashSet::new();
    for url in ranked_urls {
        if citations.len() >= MAX_FETCH_PAGES {
            break;
        }
        if !seen_urls.insert(url.clone()) {
            continue;
        }
        let content = fetch_doc_content(
            pool,
            host_id,
            local_docs_root.as_deref(),
            &url,
            &scope_key,
            &mut cache,
            &mut telemetry,
            client.as_ref(),
        )
        .await;
        let Some(content) = content else {
            continue;
        };
        let (section, snippet) = extract_doc_snippet(&content, &keywords);
        if snippet.is_empty() {
            continue;
        }
        citations.push(DocCitation {
            url: normalize_doc_url(&url),
            section,
        });
        snippets.push(snippet);
    }

    if citations.is_empty() && !rule_matches.is_empty() {
        for item in rule_matches.iter().take(2) {
            if let Some(primary_hint) = item.rule.hints.first() {
                citations.push(DocCitation {
                    url: hint_to_url(primary_hint),
                    section: "Overview".to_string(),
                });
            }
        }
    }

    let hypotheses = build_hypotheses(&rule_matches, &snippets);
    let fix_steps = build_fix_steps(&rule_matches);
    let confidence = calculate_confidence(
        &rule_matches,
        &citations,
        local_docs_root.is_some(),
        telemetry.fallback_used,
    );
    let version_awareness = build_version_awareness(
        request.openclaw_version.as_deref(),
        local_docs_root.is_some(),
    );
    let status = if hypotheses.is_empty() && citations.is_empty() {
        "unavailable".to_string()
    } else {
        "ok".to_string()
    };

    let guidance = DocGuidance {
        status,
        source_strategy: "local-first".to_string(),
        root_cause_hypotheses: if hypotheses.is_empty() {
            vec![RootCauseHypothesis {
                title: "Insufficient diagnostic signals".to_string(),
                reason: "No strong signature match; fallback guidance generated from generic troubleshooting flow."
                    .to_string(),
                score: 0.3,
            }]
        } else {
            hypotheses
        },
        fix_steps: if fix_steps.is_empty() {
            vec![
                "Run `openclaw doctor --json` and capture the first actionable error.".to_string(),
                "Run `openclaw gateway status` and verify gateway endpoint/port/token settings."
                    .to_string(),
                "Use `clawpal doctor config-read` to inspect the failing config subtree before changing values."
                    .to_string(),
            ]
        } else {
            fix_steps
        },
        confidence,
        citations,
        version_awareness,
        resolver_meta: ResolverMeta {
            cache_hit: telemetry.cache_hit,
            sources_checked: telemetry.sources_checked,
            rules_matched: telemetry.rules_matched,
            fetched_pages: telemetry.fetched_pages,
            fallback_used: telemetry.fallback_used,
        },
    };

    save_cache(paths, &cache);
    guidance
}

fn build_signature_text(request: &DocResolveRequest) -> String {
    let mut parts = Vec::new();
    parts.push(request.config_content.clone());
    parts.push(request.error_log.clone());
    if let Some(status) = &request.gateway_status {
        parts.push(status.clone());
    }
    for issue in &request.doctor_issues {
        parts.push(format!("{} {} {}", issue.id, issue.severity, issue.message));
    }
    parts.join("\n").to_ascii_lowercase()
}

fn match_route_rules(signature: &str) -> Vec<RuleMatch> {
    let mut out = Vec::new();
    for rule in ROUTE_RULES {
        let hits = rule
            .keywords
            .iter()
            .filter(|kw| signature.contains(**kw))
            .count();
        if hits == 0 {
            continue;
        }
        let score = (rule.weight + (hits as f32) * 0.07).min(0.99);
        out.push(RuleMatch { rule, score });
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

fn top_keywords(matches: &[RuleMatch]) -> Vec<String> {
    let mut out = Vec::new();
    for item in matches.iter().take(3) {
        for keyword in item.rule.keywords.iter().take(4) {
            out.push((*keyword).to_string());
        }
    }
    out
}

fn rank_urls_from_rules_and_index(
    matches: &[RuleMatch],
    index_links: &[DocLink],
    keywords: &[String],
) -> Vec<String> {
    let mut score_map: HashMap<String, f32> = HashMap::new();
    for item in matches {
        for hint in item.rule.hints {
            let url = hint_to_url(hint);
            let hint_lc = hint.to_ascii_lowercase();
            let mut keyword_bonus = 0.0f32;
            for keyword in keywords {
                let needle = keyword.trim().to_ascii_lowercase();
                if !needle.is_empty() && hint_lc.contains(&needle) {
                    keyword_bonus += 0.06;
                }
            }
            *score_map.entry(url).or_insert(0.0) += item.score + 0.1 + keyword_bonus;
        }
    }

    for link in index_links {
        let mut score = 0.05;
        let url_lc = link.url.to_ascii_lowercase();
        let title_lc = link.title.to_ascii_lowercase();
        for item in matches {
            for hint in item.rule.hints {
                if url_lc.contains(&hint.trim_matches('/').to_ascii_lowercase()) {
                    score += item.score;
                }
            }
        }
        for keyword in keywords {
            if url_lc.contains(keyword) || title_lc.contains(keyword) {
                score += 0.05;
            }
        }
        *score_map.entry(link.url.clone()).or_insert(0.0) += score;
    }

    let mut pairs = score_map.into_iter().collect::<Vec<_>>();
    pairs.sort_by(|a, b| {
        let by_score = b.1.total_cmp(&a.1);
        if by_score == std::cmp::Ordering::Equal {
            a.0.cmp(&b.0)
        } else {
            by_score
        }
    });
    pairs.into_iter().map(|(url, _)| url).collect()
}

fn build_hypotheses(matches: &[RuleMatch], snippets: &[String]) -> Vec<RootCauseHypothesis> {
    let mut out = Vec::new();
    for item in matches.iter().take(3) {
        let snippet_hint = snippets
            .iter()
            .find(|snippet| {
                item.rule
                    .keywords
                    .iter()
                    .any(|keyword| snippet.to_ascii_lowercase().contains(keyword))
            })
            .map(|snippet| truncate_for_reason(snippet, 180))
            .unwrap_or_default();
        let reason = if snippet_hint.is_empty() {
            item.rule.reason.to_string()
        } else {
            format!("{} Evidence: {}", item.rule.reason, snippet_hint)
        };
        out.push(RootCauseHypothesis {
            title: item.rule.title.to_string(),
            reason,
            score: item.score,
        });
    }
    out
}

fn build_fix_steps(matches: &[RuleMatch]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in matches.iter().take(2) {
        for step in item.rule.fix_steps {
            if seen.insert(step.to_string()) {
                out.push((*step).to_string());
            }
        }
    }
    out.truncate(8);
    out
}

fn calculate_confidence(
    matches: &[RuleMatch],
    citations: &[DocCitation],
    has_local_docs: bool,
    used_fallback: bool,
) -> f32 {
    let mut score = if matches.is_empty() { 0.22 } else { 0.4 };
    if has_local_docs {
        score += 0.18;
    }
    score += (citations.len().min(3) as f32) * 0.11;
    if used_fallback {
        score -= 0.06;
    }
    score.clamp(0.1, 0.95)
}

fn build_version_awareness(version: Option<&str>, used_local_docs: bool) -> String {
    match (version, used_local_docs) {
        (Some(v), true) if !v.trim().is_empty() => format!(
            "Prefer target-host local docs aligned with installed OpenClaw version `{}`.",
            v.trim()
        ),
        (Some(v), false) if !v.trim().is_empty() => format!(
            "Target local docs unavailable; fallback to docs.openclaw.ai for OpenClaw version `{}` context.",
            v.trim()
        ),
        (_, true) => "Prefer target-host local docs; version string unavailable.".to_string(),
        _ => "Target local docs unavailable; fallback to docs.openclaw.ai index surfaces.".to_string(),
    }
}

fn hint_to_url(hint: &str) -> String {
    if hint.starts_with("http://") || hint.starts_with("https://") {
        return normalize_doc_url(hint);
    }
    let normalized = hint.trim_start_matches('/').trim_end_matches(".md");
    format!("https://docs.openclaw.ai/{normalized}")
}

fn normalize_doc_url(url: &str) -> String {
    url.trim().trim_end_matches('#').to_string()
}

async fn fetch_doc_content(
    pool: Option<&SshConnectionPool>,
    host_id: Option<&str>,
    docs_root: Option<&str>,
    url: &str,
    scope_key: &str,
    cache: &mut ResolverCache,
    telemetry: &mut ResolveTelemetry,
    client: Option<&Client>,
) -> Option<String> {
    if let Some(root) = docs_root {
        let rel_candidates = url_to_relpath_candidates(url);
        match (pool, host_id) {
            (Some(p), Some(host)) => {
                for rel in rel_candidates {
                    let remote_path = format!("{}/{}", root.trim_end_matches('/'), rel);
                    if let Some(content) =
                        cached_remote_file(p, host, &remote_path, scope_key, cache, telemetry).await
                    {
                        telemetry.fetched_pages += 1;
                        return Some(content);
                    }
                }
            }
            _ => {
                let root_path = Path::new(root);
                for rel in rel_candidates {
                    let local_path = root_path.join(&rel);
                    if let Some(content) =
                        cached_local_file(&local_path, scope_key, cache, telemetry)
                    {
                        telemetry.fetched_pages += 1;
                        return Some(content);
                    }
                }
            }
        }
    }

    let client_ref = client?;
    let content = cached_http_get(client_ref, url, "page", scope_key, cache, telemetry).await?;
    telemetry.fetched_pages += 1;
    Some(content)
}

fn url_to_relpath_candidates(url: &str) -> Vec<String> {
    let mut out = Vec::new();
    let path = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split_once('/').map(|(_, path)| path))
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .trim_start_matches('/')
        .trim();
    if path.is_empty() {
        return out;
    }
    if path.contains("..") {
        return out;
    }
    out.push(path.to_string());
    if !path.ends_with(".md") && !path.ends_with(".mdx") && !path.ends_with(".txt") {
        out.push(format!("{path}.md"));
        out.push(format!("{path}.mdx"));
        out.push(format!("{path}/index.md"));
    } else if path.ends_with(".md") {
        out.push(path.trim_end_matches(".md").to_string());
    }
    out
}

fn discover_local_docs_root() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("/usr/lib/node_modules/openclaw/docs"),
        PathBuf::from("/usr/local/lib/node_modules/openclaw/docs"),
        PathBuf::from("/opt/homebrew/lib/node_modules/openclaw/docs"),
    ];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".npm-global/lib/node_modules/openclaw/docs"));
    }
    candidates.into_iter().find(|path| path.is_dir())
}

async fn discover_remote_docs_root(pool: &SshConnectionPool, host_id: &str) -> Option<String> {
    let script = "for d in \
\"/usr/lib/node_modules/openclaw/docs\" \
\"/usr/local/lib/node_modules/openclaw/docs\" \
\"/opt/homebrew/lib/node_modules/openclaw/docs\" \
\"$HOME/.npm-global/lib/node_modules/openclaw/docs\"; \
do [ -d \"$d\" ] && printf \"%s\" \"$d\" && break; done";
    let output = pool.exec_login(host_id, script).await.ok()?;
    let root = output.stdout.trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(root)
    }
}

fn extract_doc_snippet(content: &str, keywords: &[String]) -> (String, String) {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return ("Overview".to_string(), String::new());
    }
    let mut target_idx = 0usize;
    if !keywords.is_empty() {
        for (idx, line) in lines.iter().enumerate() {
            let lowered = line.to_ascii_lowercase();
            if keywords.iter().any(|keyword| lowered.contains(keyword)) {
                target_idx = idx;
                break;
            }
        }
    }

    let mut section = "Overview".to_string();
    for idx in (0..=target_idx).rev() {
        let line = lines[idx].trim();
        if line.starts_with('#') {
            let title = line.trim_start_matches('#').trim();
            if !title.is_empty() {
                section = title.to_string();
                break;
            }
        }
    }

    let start = target_idx.saturating_sub(2);
    let end = (target_idx + 9).min(lines.len());
    let snippet = lines[start..end].join("\n").trim().to_string();
    (section, snippet)
}

fn truncate_for_reason(raw: &str, max_chars: usize) -> String {
    let trimmed = raw.replace('\n', " ").trim().to_string();
    if trimmed.chars().count() <= max_chars {
        return trimmed;
    }
    let shortened = trimmed.chars().take(max_chars).collect::<String>();
    format!("{shortened}...")
}

async fn cached_http_get(
    client: &Client,
    url: &str,
    kind: &str,
    scope_key: &str,
    cache: &mut ResolverCache,
    telemetry: &mut ResolveTelemetry,
) -> Option<String> {
    let cache_key = format!("scope:{scope_key}:{kind}:{}", normalize_doc_url(url));
    if let Some(text) = cache_get(cache, &cache_key) {
        telemetry.cache_hit = true;
        return Some(text);
    }
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    cache_put(cache, cache_key, text.clone());
    Some(text)
}

fn cached_local_file(
    path: &Path,
    scope_key: &str,
    cache: &mut ResolverCache,
    telemetry: &mut ResolveTelemetry,
) -> Option<String> {
    if !path.exists() || !path.is_file() {
        return None;
    }
    let key = format!("scope:{scope_key}:local:{}", path.to_string_lossy());
    if let Some(text) = cache_get(cache, &key) {
        telemetry.cache_hit = true;
        return Some(text);
    }
    let text = fs::read_to_string(path).ok()?;
    cache_put(cache, key, text.clone());
    Some(text)
}

async fn cached_remote_file(
    pool: &SshConnectionPool,
    host_id: &str,
    remote_path: &str,
    scope_key: &str,
    cache: &mut ResolverCache,
    telemetry: &mut ResolveTelemetry,
) -> Option<String> {
    let key = format!("scope:{scope_key}:remote:{host_id}:{remote_path}");
    if let Some(text) = cache_get(cache, &key) {
        telemetry.cache_hit = true;
        return Some(text);
    }
    let text = pool.sftp_read(host_id, remote_path).await.ok()?;
    cache_put(cache, key, text.clone());
    Some(text)
}

fn parse_llms_links(raw: &str) -> Vec<DocLink> {
    let mut out = Vec::new();
    let Ok(link_re) = Regex::new(r"\[([^\]]*)\]\((https?://[^)\s]+)\)") else {
        return out;
    };
    for cap in link_re.captures_iter(raw) {
        let title = cap
            .get(1)
            .map(|m| m.as_str().trim())
            .unwrap_or("")
            .to_string();
        let url = cap
            .get(2)
            .map(|m| m.as_str().trim())
            .unwrap_or("")
            .to_string();
        if url.is_empty() {
            continue;
        }
        out.push(DocLink {
            title: if title.is_empty() || title == "null" {
                "OpenClaw Doc".to_string()
            } else {
                title
            },
            url: normalize_doc_url(&url),
        });
    }
    out
}

fn parse_sitemap_links(raw: &str) -> Vec<DocLink> {
    let mut out = Vec::new();
    let Ok(loc_re) = Regex::new(r"<loc>([^<]+)</loc>") else {
        return out;
    };
    for cap in loc_re.captures_iter(raw) {
        let Some(url_match) = cap.get(1) else {
            continue;
        };
        let url = normalize_doc_url(url_match.as_str());
        if url.is_empty() {
            continue;
        }
        let title = url
            .split('/')
            .next_back()
            .map(|chunk| chunk.replace('-', " "))
            .filter(|chunk| !chunk.is_empty())
            .unwrap_or_else(|| "OpenClaw Doc".to_string());
        out.push(DocLink { title, url });
    }
    out
}

fn parse_llms_full_sources(raw: &str) -> Vec<DocLink> {
    let mut out = Vec::new();
    let Ok(source_re) = Regex::new(r"(?m)^Source:\s*(https?://\S+)\s*$") else {
        return out;
    };
    for cap in source_re.captures_iter(raw) {
        let Some(url_match) = cap.get(1) else {
            continue;
        };
        let url = normalize_doc_url(url_match.as_str());
        if url.is_empty() {
            continue;
        }
        let title = url
            .split('/')
            .next_back()
            .map(|chunk| chunk.replace('-', " "))
            .filter(|chunk| !chunk.is_empty())
            .unwrap_or_else(|| "OpenClaw Doc".to_string());
        out.push(DocLink { title, url });
    }
    out
}

fn cache_path(paths: &OpenClawPaths) -> PathBuf {
    paths.clawpal_dir.join("openclaw-doc-resolver-cache.json")
}

fn load_cache(paths: &OpenClawPaths) -> ResolverCache {
    let path = cache_path(paths);
    let Ok(raw) = fs::read_to_string(path) else {
        return ResolverCache {
            schema_version: CACHE_SCHEMA_VERSION,
            ..ResolverCache::default()
        };
    };
    let Ok(mut parsed) = serde_json::from_str::<ResolverCache>(&raw) else {
        return ResolverCache {
            schema_version: CACHE_SCHEMA_VERSION,
            ..ResolverCache::default()
        };
    };
    if parsed.schema_version != CACHE_SCHEMA_VERSION {
        parsed = ResolverCache {
            schema_version: CACHE_SCHEMA_VERSION,
            ..ResolverCache::default()
        };
    }
    parsed
}

fn save_cache(paths: &OpenClawPaths, cache: &ResolverCache) {
    let path = cache_path(paths);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(cache) {
        let _ = fs::write(path, raw);
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn cache_get(cache: &mut ResolverCache, key: &str) -> Option<String> {
    let now = now_unix();
    let entry = cache.entries.get(key)?;
    if now.saturating_sub(entry.fetched_at) > CACHE_TTL_SECONDS {
        cache.entries.remove(key);
        return None;
    }
    Some(entry.content.clone())
}

fn cache_put(cache: &mut ResolverCache, key: String, content: String) {
    cache.entries.insert(
        key,
        CachedEntry {
            fetched_at: now_unix(),
            content,
        },
    );
}

fn invalidate_cache_if_version_changed(
    cache: &mut ResolverCache,
    scope_key: &str,
    version: Option<&str>,
) {
    let next_version = version.unwrap_or("unknown").trim().to_string();
    let previous = cache.scope_version.get(scope_key).cloned();
    if previous.as_deref() == Some(next_version.as_str()) {
        return;
    }
    let prefix = format!("scope:{scope_key}:");
    cache.entries.retain(|key, _| !key.starts_with(&prefix));
    cache
        .scope_version
        .insert(scope_key.to_string(), next_version);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_has_first_batch_rule_coverage() {
        assert!(
            ROUTE_RULES.len() >= 10,
            "router should include at least 10 signature rules"
        );
    }

    #[test]
    fn parse_llms_links_extracts_markdown_links() {
        let raw = "- [Groups](https://docs.openclaw.ai/channels/groups.md)\n- [null](https://docs.openclaw.ai/auth-credential-semantics.md)";
        let links = parse_llms_links(raw);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "https://docs.openclaw.ai/channels/groups.md");
        assert_eq!(links[1].title, "OpenClaw Doc");
    }

    #[test]
    fn parse_sitemap_links_extracts_loc_urls() {
        let raw = r#"<?xml version="1.0"?><urlset><url><loc>https://docs.openclaw.ai/automation/cron-vs-heartbeat</loc></url></urlset>"#;
        let links = parse_sitemap_links(raw);
        assert_eq!(links.len(), 1);
        assert_eq!(
            links[0].url,
            "https://docs.openclaw.ai/automation/cron-vs-heartbeat"
        );
    }

    #[test]
    fn parse_llms_full_sources_extracts_source_lines() {
        let raw = "# X\nSource: https://docs.openclaw.ai/cli/gateway\n\n# Y\nSource: https://docs.openclaw.ai/security";
        let links = parse_llms_full_sources(raw);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].url, "https://docs.openclaw.ai/cli/gateway");
    }

    #[test]
    fn route_match_detects_provider_auth_failures() {
        let signature = "unauthorized invalid api key auth_ref missing provider not found";
        let matches = match_route_rules(signature);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].rule.id, "provider_auth_failure");
    }

    #[test]
    fn version_change_invalidates_scope_cache() {
        let mut cache = ResolverCache {
            schema_version: CACHE_SCHEMA_VERSION,
            scope_version: HashMap::from([("local".to_string(), "1.0.0".to_string())]),
            entries: HashMap::from([(
                "scope:local:page:https://docs.openclaw.ai/cli/gateway".to_string(),
                CachedEntry {
                    fetched_at: now_unix(),
                    content: "cached".to_string(),
                },
            )]),
        };
        invalidate_cache_if_version_changed(&mut cache, "local", Some("1.1.0"));
        assert!(cache
            .entries
            .keys()
            .all(|key| !key.starts_with("scope:local:")));
        assert_eq!(
            cache.scope_version.get("local").map(String::as_str),
            Some("1.1.0")
        );
    }

    #[test]
    fn hint_to_url_normalizes_non_http_hint() {
        assert_eq!(
            hint_to_url("cli/gateway"),
            "https://docs.openclaw.ai/cli/gateway"
        );
        assert_eq!(
            hint_to_url("cli/gateway.md"),
            "https://docs.openclaw.ai/cli/gateway"
        );
        assert_eq!(
            hint_to_url("https://docs.openclaw.ai/security"),
            "https://docs.openclaw.ai/security"
        );
    }

    #[test]
    fn url_to_relpath_candidates_expands_markdown_and_index() {
        let candidates =
            url_to_relpath_candidates("https://docs.openclaw.ai/automation/cron-vs-heartbeat");
        assert!(candidates.contains(&"automation/cron-vs-heartbeat".to_string()));
        assert!(candidates.contains(&"automation/cron-vs-heartbeat.md".to_string()));
        assert!(candidates.contains(&"automation/cron-vs-heartbeat/index.md".to_string()));
    }

    #[test]
    fn url_to_relpath_candidates_rejects_traversal() {
        let candidates = url_to_relpath_candidates("https://docs.openclaw.ai/../../etc/passwd");
        assert!(candidates.is_empty());
    }

    #[test]
    fn extract_doc_snippet_picks_nearest_heading_and_keyword_block() {
        let doc = r#"
# Gateway
General details

## Troubleshooting
foo
gateway token mismatch detected
bar
"#;
        let (section, snippet) = extract_doc_snippet(doc, &["token mismatch".to_string()]);
        assert_eq!(section, "Troubleshooting");
        assert!(snippet.contains("gateway token mismatch detected"));
    }

    #[test]
    fn build_fix_steps_dedupes_and_limits_output() {
        let rule_a = &ROUTE_RULES[0];
        let rule_b = &ROUTE_RULES[1];
        let matches = vec![
            RuleMatch {
                rule: rule_a,
                score: 0.9,
            },
            RuleMatch {
                rule: rule_b,
                score: 0.8,
            },
        ];
        let steps = build_fix_steps(&matches);
        assert!(!steps.is_empty());
        assert!(steps.len() <= 8);
        let uniq = steps.iter().collect::<HashSet<_>>();
        assert_eq!(uniq.len(), steps.len());
    }

    #[test]
    fn calculate_confidence_respects_bounds() {
        let empty = calculate_confidence(&[], &[], false, true);
        assert!(empty >= 0.1);
        let rich = calculate_confidence(
            &[RuleMatch {
                rule: &ROUTE_RULES[0],
                score: 0.98,
            }],
            &[
                DocCitation {
                    url: "https://docs.openclaw.ai/a".to_string(),
                    section: "A".to_string(),
                },
                DocCitation {
                    url: "https://docs.openclaw.ai/b".to_string(),
                    section: "B".to_string(),
                },
                DocCitation {
                    url: "https://docs.openclaw.ai/c".to_string(),
                    section: "C".to_string(),
                },
            ],
            true,
            false,
        );
        assert!(rich <= 0.95);
        assert!(rich > empty);
    }

    #[test]
    fn rank_urls_from_rules_and_index_prioritizes_rule_hints() {
        let matches = vec![RuleMatch {
            rule: &ROUTE_RULES[2],
            score: 0.9,
        }];
        let index = vec![
            DocLink {
                title: "Gateway".to_string(),
                url: "https://docs.openclaw.ai/cli/gateway.md".to_string(),
            },
            DocLink {
                title: "Other".to_string(),
                url: "https://docs.openclaw.ai/channels/groups.md".to_string(),
            },
        ];
        let ranked = rank_urls_from_rules_and_index(&matches, &index, &["gateway".to_string()]);
        assert!(!ranked.is_empty());
        assert!(
            ranked[0] == "https://docs.openclaw.ai/cli/gateway"
                || ranked[0] == "https://docs.openclaw.ai/cli/gateway.md"
        );
    }

    #[test]
    fn cache_get_expires_entries_by_ttl() {
        let mut cache = ResolverCache {
            schema_version: CACHE_SCHEMA_VERSION,
            scope_version: HashMap::new(),
            entries: HashMap::from([(
                "k".to_string(),
                CachedEntry {
                    fetched_at: now_unix().saturating_sub(CACHE_TTL_SECONDS + 5),
                    content: "x".to_string(),
                },
            )]),
        };
        let hit = cache_get(&mut cache, "k");
        assert!(hit.is_none());
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn build_version_awareness_mentions_fallback_when_local_missing() {
        let msg = build_version_awareness(Some("2026.3.1"), false);
        assert!(msg.contains("fallback"));
        assert!(msg.contains("2026.3.1"));
    }
}
