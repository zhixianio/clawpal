use regex::Regex;
use std::sync::OnceLock;

fn sk_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bsk-[A-Za-z0-9_-]{8,}\b").expect("valid sk token regex"))
}

fn bearer_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+\b").expect("valid bearer token regex")
    })
}

fn key_value_secret_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(token|api[_-]?key|password)\s*=\s*([^&\s]+)")
            .expect("valid key=value secret regex")
    })
}

fn email_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b").expect("valid email regex")
    })
}

fn unix_home_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(/Users|/home)/[^/\s]+").expect("valid unix home regex"))
}

fn windows_home_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)[A-Z]:\\Users\\[^\\\s]+").expect("valid windows home regex")
    })
}

fn ssh_user_host_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\b").expect("valid ssh user@host regex")
    })
}

fn ipv4_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b",
        )
        .expect("valid ipv4 regex")
    })
}

fn chat_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(discord(?:_user)?_?id|telegram(?:_user|_chat)?_?id|chat_id|user_id)\s*[:=]\s*\d{5,}\b",
        )
        .expect("valid chat/user id regex")
    })
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)\bhttps?://[^\s"'<>]+"#).expect("valid url regex"))
}

pub fn sanitize_text(raw: &str) -> String {
    let mut out = raw.to_string();
    out = sk_token_re().replace_all(&out, "<REDACTED_API_KEY>").into_owned();
    out = bearer_token_re()
        .replace_all(&out, "Bearer <REDACTED_TOKEN>")
        .into_owned();
    out = key_value_secret_re()
        .replace_all(&out, "$1=<REDACTED>")
        .into_owned();
    out = email_re().replace_all(&out, "<REDACTED_EMAIL>").into_owned();
    out = unix_home_re().replace_all(&out, "<HOME>").into_owned();
    out = windows_home_re().replace_all(&out, "<HOME>").into_owned();
    out = ssh_user_host_re()
        .replace_all(&out, "<REDACTED_SSH_TARGET>")
        .into_owned();
    out = ipv4_re().replace_all(&out, "<REDACTED_IP>").into_owned();
    out = chat_id_re()
        .replace_all(&out, "$1=<REDACTED_ID>")
        .into_owned();
    url_re().replace_all(&out, "<REDACTED_URL>").into_owned()
}

pub fn sanitize_optional_text(raw: Option<&str>) -> Option<String> {
    raw.map(sanitize_text).filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_keys_and_bearer_tokens() {
        let input = "token sk-abcdEFGH12345678 Bearer abc.def.ghi";
        let output = sanitize_text(input);
        assert!(!output.contains("sk-abcdEFGH12345678"));
        assert!(!output.contains("abc.def.ghi"));
        assert!(output.contains("<REDACTED_API_KEY>"));
        assert!(output.contains("Bearer <REDACTED_TOKEN>"));
    }

    #[test]
    fn redacts_key_value_secrets() {
        let input = "token=abc123 api_key = zzz password=passw0rd";
        let output = sanitize_text(input);
        assert_eq!(output, "token=<REDACTED> api_key=<REDACTED> password=<REDACTED>");
    }

    #[test]
    fn redacts_email_and_home_paths() {
        let input = "mail me at dev@example.com, path /Users/alice/.openclaw and C:\\Users\\bob\\AppData";
        let output = sanitize_text(input);
        assert!(!output.contains("dev@example.com"));
        assert!(!output.contains("/Users/alice"));
        assert!(!output.contains("C:\\Users\\bob"));
        assert!(output.contains("<REDACTED_EMAIL>"));
        assert!(output.contains("<HOME>"));
    }

    #[test]
    fn redacts_ssh_targets_and_ips() {
        let input = "ssh root@prod.internal from 10.24.1.88";
        let output = sanitize_text(input);
        assert!(!output.contains("root@prod.internal"));
        assert!(!output.contains("10.24.1.88"));
        assert!(output.contains("<REDACTED_SSH_TARGET>"));
        assert!(output.contains("<REDACTED_IP>"));
    }

    #[test]
    fn redacts_chat_ids_and_urls() {
        let input =
            "discord_user_id=123456789 telegram_chat_id:987654321 endpoint=https://internal.example.com/v1";
        let output = sanitize_text(input);
        assert!(output.contains("discord_user_id=<REDACTED_ID>"));
        assert!(output.contains("telegram_chat_id=<REDACTED_ID>"));
        assert!(output.contains("<REDACTED_URL>"));
    }

    #[test]
    fn sanitize_optional_text_handles_none_and_empty() {
        assert_eq!(sanitize_optional_text(None), None);
        assert_eq!(sanitize_optional_text(Some("  ")), None);
        assert_eq!(
            sanitize_optional_text(Some("email a@b.com")),
            Some("email <REDACTED_EMAIL>".to_string())
        );
    }
}
