use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::openclaw::OpenclawCli;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub auth_ref: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("provider and model are required")]
    InvalidProfile,
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseFile {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize profiles: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("openclaw command failed: {0}")]
    Openclaw(String),
}

pub type Result<T> = std::result::Result<T, ProfileError>;

pub fn list_profiles(_openclaw: &OpenclawCli) -> Result<Vec<ModelProfile>> {
    load_profiles()
}

pub fn upsert_profile(_openclaw: &OpenclawCli, mut profile: ModelProfile) -> Result<ModelProfile> {
    if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
        return Err(ProfileError::InvalidProfile);
    }
    if profile.id.trim().is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    if profile.name.trim().is_empty() {
        profile.name = format!("{}/{}", profile.provider, profile.model);
    }

    let mut profiles = load_profiles()?;
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
        *existing = profile.clone();
    } else {
        profiles.push(profile.clone());
    }
    save_profiles(&profiles)?;
    Ok(profile)
}

pub fn delete_profile(_openclaw: &OpenclawCli, id: &str) -> Result<bool> {
    let mut profiles = load_profiles()?;
    let before = profiles.len();
    profiles.retain(|p| p.id != id);
    let removed = profiles.len() != before;
    if removed {
        save_profiles(&profiles)?;
    }
    Ok(removed)
}

pub fn test_profile(openclaw: &OpenclawCli, id: &str) -> Result<TestResult> {
    let profiles = load_profiles()?;
    let Some(profile) = profiles.iter().find(|p| p.id == id) else {
        return Ok(TestResult {
            ok: false,
            message: format!("profile '{id}' not found"),
        });
    };
    let output = openclaw
        .run(&["models", "list", "--all", "--json"])
        .map_err(|e| ProfileError::Openclaw(e.to_string()))?;
    if output.exit_code != 0 {
        let err = output.stderr.trim();
        return Ok(TestResult {
            ok: false,
            message: if err.is_empty() {
                format!("{} (probe failed with exit code {})", profile.name, output.exit_code)
            } else {
                format!("{} ({err})", profile.name)
            },
        });
    }
    let listed = model_is_listed(&output.stdout, &profile.provider, &profile.model);
    Ok(TestResult {
        ok: listed,
        message: if listed {
            format!("{} (model available)", profile.name)
        } else {
            format!(
                "{} (model '{}' not found in openclaw models list)",
                profile.name, profile.model
            )
        },
    })
}

pub fn list_profiles_from_storage_json(content: &str) -> Vec<ModelProfile> {
    parse_storage_lenient(content).profiles
}

pub fn find_profile_in_storage_json(content: &str, profile_id: &str) -> Result<Option<ModelProfile>> {
    let storage = parse_storage_lenient(content);
    Ok(storage
        .profiles
        .into_iter()
        .find(|profile| profile.id == profile_id))
}

pub fn upsert_profile_in_storage_json(
    content: &str,
    mut profile: ModelProfile,
) -> Result<(ModelProfile, String)> {
    if profile.provider.trim().is_empty() || profile.model.trim().is_empty() {
        return Err(ProfileError::InvalidProfile);
    }
    if profile.name.trim().is_empty() {
        profile.name = format!("{}/{}", profile.provider, profile.model);
    }

    let mut storage = parse_storage_lenient(content);
    fill_profile_auth_from_existing_or_provider_donor(&mut profile, &storage.profiles);
    if profile
        .api_key
        .as_ref()
        .is_some_and(|key| !key.trim().is_empty())
        && profile.auth_ref.trim().is_empty()
    {
        profile.auth_ref = format!("{}:default", profile.provider.trim());
    }
    profile = upsert_profile_in_storage(&mut storage.profiles, profile);
    let text = serialize_storage(&storage)?;
    Ok((profile, text))
}

pub fn delete_profile_from_storage_json(content: &str, profile_id: &str) -> Result<(bool, String)> {
    let mut storage = parse_storage_lenient(content);
    let before = storage.profiles.len();
    storage.profiles.retain(|profile| profile.id != profile_id);
    let removed = storage.profiles.len() != before;
    let text = serialize_storage(&storage)?;
    Ok((removed, text))
}

fn model_is_listed(raw: &str, provider: &str, model: &str) -> bool {
    let Ok(json) = serde_json::from_str::<Value>(raw) else {
        return raw.contains(model);
    };
    model_in_value(&json, provider, model)
}

fn model_in_value(value: &Value, provider: &str, model: &str) -> bool {
    if let Some(array) = value.as_array() {
        return array
            .iter()
            .any(|entry| model_in_value(entry, provider, model));
    }

    if let Some(object) = value.as_object() {
        let provider_match = object
            .get("provider")
            .and_then(Value::as_str)
            .map(|v| v.eq_ignore_ascii_case(provider))
            .unwrap_or(false);
        let model_match = object
            .get("model")
            .or_else(|| object.get("id"))
            .and_then(Value::as_str)
            .map(|v| v == model || v.ends_with(&format!("/{model}")))
            .unwrap_or(false);
        if provider_match && model_match {
            return true;
        }
        return object
            .values()
            .any(|entry| model_in_value(entry, provider, model));
    }

    false
}

fn load_profiles() -> Result<Vec<ModelProfile>> {
    let path = profiles_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|source| ProfileError::ReadFile {
        path: path.clone(),
        source,
    })?;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Storage {
        Wrapped {
            #[serde(default)]
            profiles: Vec<ModelProfile>,
        },
        Plain(Vec<ModelProfile>),
    }
    let parsed: Storage =
        serde_json::from_str(&text).map_err(|source| ProfileError::ParseFile { path, source })?;
    Ok(match parsed {
        Storage::Wrapped { profiles } => profiles,
        Storage::Plain(profiles) => profiles,
    })
}

fn save_profiles(profiles: &[ModelProfile]) -> Result<()> {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProfileError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let json = serde_json::to_string_pretty(profiles)?;
    fs::write(&path, json).map_err(|source| ProfileError::WriteFile { path, source })?;
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ProfileStorage {
    Wrapped(ProfileStorageWrapped),
    Plain(Vec<ModelProfile>),
}

#[derive(Deserialize, Serialize)]
struct ProfileStorageWrapped {
    #[serde(default)]
    profiles: Vec<ModelProfile>,
    #[serde(default = "default_storage_version")]
    version: u8,
}

fn parse_storage_lenient(content: &str) -> ProfileStorageWrapped {
    match serde_json::from_str::<ProfileStorage>(content) {
        Ok(ProfileStorage::Wrapped(storage)) => storage,
        Ok(ProfileStorage::Plain(profiles)) => ProfileStorageWrapped {
            profiles,
            version: default_storage_version(),
        },
        Err(_) => ProfileStorageWrapped {
            profiles: Vec::new(),
            version: default_storage_version(),
        },
    }
}

fn serialize_storage(storage: &ProfileStorageWrapped) -> Result<String> {
    serde_json::to_string_pretty(storage).map_err(ProfileError::Serialize)
}

fn default_storage_version() -> u8 {
    1
}

fn fill_profile_auth_from_existing_or_provider_donor(
    profile: &mut ModelProfile,
    profiles: &[ModelProfile],
) {
    if !profile.id.trim().is_empty() {
        if let Some(existing) = profiles
            .iter()
            .find(|candidate| candidate.id == profile.id)
        {
            if profile
                .api_key
                .as_ref()
                .map_or(true, |key| key.trim().is_empty())
            {
                profile.api_key = existing.api_key.clone();
            }
            if profile.auth_ref.trim().is_empty() {
                profile.auth_ref = existing.auth_ref.clone();
            }
            return;
        }
    }

    let provider = profile.provider.trim();
    if provider.is_empty() {
        return;
    }

    if profile
        .api_key
        .as_ref()
        .map_or(true, |key| key.trim().is_empty())
    {
        if let Some(donor) = profiles.iter().find(|candidate| {
            candidate.provider.eq_ignore_ascii_case(provider)
                && candidate
                    .api_key
                    .as_ref()
                    .is_some_and(|key| !key.trim().is_empty())
        }) {
            profile.api_key = donor.api_key.clone();
        }
    }

    if profile.auth_ref.trim().is_empty() {
        if let Some(donor) = profiles.iter().find(|candidate| {
            candidate.provider.eq_ignore_ascii_case(provider) && !candidate.auth_ref.trim().is_empty()
        }) {
            profile.auth_ref = donor.auth_ref.clone();
        }
    }
}

fn upsert_profile_in_storage(profiles: &mut Vec<ModelProfile>, mut profile: ModelProfile) -> ModelProfile {
    if profile.id.trim().is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    let id = profile.id.clone();
    if let Some(existing) = profiles.iter_mut().find(|candidate| candidate.id == id) {
        *existing = profile.clone();
    } else {
        profiles.push(profile.clone());
    }
    profile
}

fn profiles_path() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAWPAL_DATA_DIR") {
        return PathBuf::from(dir).join("model-profiles.json");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".clawpal")
        .join("model-profiles.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use uuid::Uuid;

    fn temp_data_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("clawpal-core-profile-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn profile(id: &str) -> ModelProfile {
        ModelProfile {
            id: id.to_string(),
            name: "Test".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4.1".to_string(),
            auth_ref: "OPENAI_API_KEY".to_string(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        }
    }

    #[test]
    fn list_profiles_returns_saved_profiles() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let cli = OpenclawCli::with_bin("echo".to_string());
        let _ = upsert_profile(&cli, profile("p1")).expect("upsert");
        let profiles = list_profiles(&cli).expect("list");
        assert_eq!(profiles.len(), 1);
    }

    #[test]
    fn upsert_profile_saves_profile() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let cli = OpenclawCli::with_bin("echo".to_string());
        let saved = upsert_profile(&cli, profile("p2")).expect("upsert");
        assert_eq!(saved.id, "p2");
    }

    #[test]
    fn list_profiles_supports_wrapped_storage_format() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = temp_data_dir();
        std::env::set_var("CLAWPAL_DATA_DIR", &dir);
        let path = dir.join("model-profiles.json");
        let wrapped = serde_json::json!({
            "profiles": [profile("wrapped-1")],
            "version": 1
        });
        fs::write(
            &path,
            serde_json::to_string_pretty(&wrapped).expect("serialize wrapped"),
        )
        .expect("write wrapped storage");
        let cli = OpenclawCli::with_bin("echo".to_string());
        let listed = list_profiles(&cli).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "wrapped-1");
    }

    #[test]
    fn delete_profile_removes_profile() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let cli = OpenclawCli::with_bin("echo".to_string());
        let _ = upsert_profile(&cli, profile("p3")).expect("upsert");
        let removed = delete_profile(&cli, "p3").expect("delete");
        assert!(removed);
    }

    #[cfg(unix)]
    fn create_fake_openclaw_models_script(body: &str) -> String {
        let dir = temp_data_dir();
        let path = dir.join("fake-openclaw-models.sh");
        fs::write(&path, body).expect("write script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
        path.to_string_lossy().to_string()
    }

    #[test]
    #[cfg(unix)]
    fn test_profile_returns_result() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let cli = OpenclawCli::with_bin(create_fake_openclaw_models_script(
            "#!/bin/sh\nif [ \"$1\" = \"models\" ]; then echo '[{\"provider\":\"openai\",\"model\":\"gpt-4.1\"}]'; exit 0; fi\nexit 1\n",
        ));
        let _ = upsert_profile(&OpenclawCli::with_bin("echo".to_string()), profile("p4")).expect("upsert");
        let result = test_profile(&cli, "p4").expect("test");
        assert!(result.ok);
    }

    #[test]
    fn test_profile_returns_not_found_for_missing_profile() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let cli = OpenclawCli::with_bin("echo".to_string());
        let result = test_profile(&cli, "missing").expect("test");
        assert!(!result.ok);
        assert!(result.message.contains("not found"));
    }

    #[test]
    #[cfg(unix)]
    fn test_profile_reports_failure_when_openclaw_command_fails() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let _ = upsert_profile(&OpenclawCli::with_bin("echo".to_string()), profile("p5"))
            .expect("upsert");

        let dir = std::env::temp_dir().join(format!("clawpal-core-profile-fail-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let script = dir.join("fake-openclaw-fail.sh");
        fs::write(
            &script,
            "#!/bin/sh\nif [ \"$1\" = \"models\" ]; then echo 'boom' >&2; exit 9; fi\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");

        let cli = OpenclawCli::with_bin(script.to_string_lossy().to_string());
        let result = test_profile(&cli, "p5").expect("test");
        assert!(!result.ok);
    }

    #[test]
    #[cfg(unix)]
    fn test_profile_returns_false_when_model_missing() {
        let _guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CLAWPAL_DATA_DIR", temp_data_dir());
        let _ = upsert_profile(&OpenclawCli::with_bin("echo".to_string()), profile("p6")).expect("upsert");

        let cli = OpenclawCli::with_bin(create_fake_openclaw_models_script(
            "#!/bin/sh\nif [ \"$1\" = \"models\" ]; then echo '[{\"provider\":\"openai\",\"model\":\"gpt-3.5\"}]'; exit 0; fi\nexit 1\n",
        ));
        let result = test_profile(&cli, "p6").expect("test");
        assert!(!result.ok);
    }

    #[test]
    fn list_profiles_from_storage_json_supports_plain_array() {
        let content = serde_json::to_string(&vec![profile("p-json-1")]).expect("serialize");
        let listed = list_profiles_from_storage_json(&content);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "p-json-1");
    }

    #[test]
    fn upsert_profile_in_storage_json_preserves_provider_donor_auth() {
        let mut donor = profile("donor");
        donor.provider = "openai".to_string();
        donor.auth_ref = "OPENAI_API_KEY".to_string();
        donor.api_key = Some("sk-donor".to_string());
        let incoming = ModelProfile {
            id: "".to_string(),
            name: "".to_string(),
            provider: "openai".to_string(),
            model: "gpt-4.1-mini".to_string(),
            auth_ref: "".to_string(),
            api_key: None,
            base_url: None,
            description: None,
            enabled: true,
        };
        let content = serde_json::json!({ "profiles": [donor], "version": 1 }).to_string();
        let (saved, next_json) =
            upsert_profile_in_storage_json(&content, incoming).expect("upsert json");
        assert!(!saved.id.trim().is_empty());
        assert_eq!(saved.name, "openai/gpt-4.1-mini");
        assert_eq!(saved.auth_ref, "OPENAI_API_KEY");
        assert_eq!(saved.api_key.as_deref(), Some("sk-donor"));
        assert!(next_json.contains("\"version\": 1"));
    }

    #[test]
    fn delete_profile_from_storage_json_returns_true_when_removed() {
        let content = serde_json::json!({ "profiles": [profile("p-del")], "version": 1 }).to_string();
        let (removed, next_json) =
            delete_profile_from_storage_json(&content, "p-del").expect("delete json");
        assert!(removed);
        let listed = list_profiles_from_storage_json(&next_json);
        assert!(listed.is_empty());
    }

    #[test]
    fn find_profile_in_storage_json_returns_profile_when_present() {
        let content = serde_json::json!({ "profiles": [profile("p-find")], "version": 1 }).to_string();
        let found = find_profile_in_storage_json(&content, "p-find")
            .expect("find profile")
            .expect("profile present");
        assert_eq!(found.id, "p-find");
    }
}
