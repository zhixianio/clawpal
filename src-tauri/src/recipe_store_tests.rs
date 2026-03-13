use crate::recipe_store::{Artifact, RecipeStore, ResourceClaim, Run};

fn sample_run() -> Run {
    Run {
        id: "run_01".into(),
        instance_id: "inst_01".into(),
        recipe_id: "discord-channel-persona".into(),
        execution_kind: "attachment".into(),
        runner: "local".into(),
        status: "succeeded".into(),
        summary: "Applied persona patch".into(),
        started_at: "2026-03-11T10:00:00Z".into(),
        finished_at: Some("2026-03-11T10:00:03Z".into()),
        artifacts: vec![Artifact {
            id: "artifact_01".into(),
            kind: "configDiff".into(),
            label: "Rendered patch".into(),
            path: Some("/tmp/rendered-patch.json".into()),
        }],
        resource_claims: vec![ResourceClaim {
            kind: "path".into(),
            id: Some("openclaw.config".into()),
            target: None,
            path: Some("~/.openclaw/openclaw.json".into()),
        }],
        warnings: vec![],
        source_origin: None,
        source_digest: None,
        workspace_path: None,
    }
}

fn sample_run_with_source() -> Run {
    let mut run = sample_run();
    run.source_origin = Some("draft".into());
    run.source_digest = Some("digest-123".into());
    run.workspace_path =
        Some("/Users/chen/.clawpal/recipes/workspace/channel-persona.recipe.json".into());
    run
}

#[test]
fn record_run_persists_instance_and_artifacts() {
    let store = RecipeStore::for_test();
    let run = store.record_run(sample_run()).expect("record run");

    assert_eq!(store.list_runs("inst_01").expect("list runs")[0].id, run.id);
    assert_eq!(
        store.list_instances().expect("list instances")[0]
            .last_run_id
            .as_deref(),
        Some(run.id.as_str())
    );
    assert_eq!(
        store.list_runs("inst_01").expect("list runs")[0].artifacts[0].id,
        "artifact_01"
    );
}

#[test]
fn list_all_runs_returns_latest_runs() {
    let store = RecipeStore::for_test();
    store.record_run(sample_run()).expect("record first run");

    let mut second_run = sample_run();
    second_run.id = "run_02".into();
    second_run.instance_id = "ssh:prod-a".into();
    second_run.started_at = "2026-03-11T11:00:00Z".into();
    second_run.finished_at = Some("2026-03-11T11:00:05Z".into());
    store.record_run(second_run).expect("record second run");

    let runs = store.list_all_runs().expect("list all runs");
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].id, "run_02");
    assert_eq!(runs[1].id, "run_01");
}

#[test]
fn recorded_run_persists_source_digest_and_origin() {
    let store = RecipeStore::for_test();
    store
        .record_run(sample_run_with_source())
        .expect("record run with source");

    let stored = store.list_runs("inst_01").expect("list runs");
    assert_eq!(stored[0].source_origin.as_deref(), Some("draft"));
    assert_eq!(stored[0].source_digest.as_deref(), Some("digest-123"));
    assert!(stored[0]
        .workspace_path
        .as_deref()
        .is_some_and(|path| path.ends_with("channel-persona.recipe.json")));
}

#[test]
fn delete_runs_for_instance_removes_runs_and_rebuilds_instances() {
    let store = RecipeStore::for_test();
    store.record_run(sample_run()).expect("record first run");

    let mut second_run = sample_run();
    second_run.id = "run_02".into();
    second_run.instance_id = "ssh:prod-a".into();
    second_run.started_at = "2026-03-11T11:00:00Z".into();
    second_run.finished_at = Some("2026-03-11T11:00:05Z".into());
    store.record_run(second_run).expect("record second run");

    let deleted = store
        .delete_runs(Some("inst_01"))
        .expect("delete instance runs");

    assert_eq!(deleted, 1);
    assert!(store
        .list_runs("inst_01")
        .expect("list removed runs")
        .is_empty());
    let remaining_runs = store.list_all_runs().expect("list all runs");
    assert_eq!(remaining_runs.len(), 1);
    assert_eq!(remaining_runs[0].instance_id, "ssh:prod-a");
    let instances = store.list_instances().expect("list instances");
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].id, "ssh:prod-a");
    assert_eq!(instances[0].last_run_id.as_deref(), Some("run_02"));
}

#[test]
fn delete_runs_without_scope_clears_all_runs_and_instances() {
    let store = RecipeStore::for_test();
    store.record_run(sample_run()).expect("record first run");

    let deleted = store.delete_runs(None).expect("delete all runs");

    assert_eq!(deleted, 1);
    assert!(store.list_all_runs().expect("list all runs").is_empty());
    assert!(store.list_instances().expect("list instances").is_empty());
}
