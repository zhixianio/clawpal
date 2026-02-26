#[cfg(target_os = "windows")]
mod windows_wsl2_tests {
    use std::collections::HashMap;

    use clawpal::install::runners::wsl2;
    use clawpal::install::types::InstallStep;

    #[test]
    #[ignore = "requires a Windows host with WSL2 available"]
    fn wsl2_precheck_runs_on_windows() {
        let out = wsl2::run_step(&InstallStep::Precheck, &HashMap::new())
            .expect("wsl2 precheck should run on windows");
        assert!(out.summary.contains("wsl2 precheck"));
        assert!(!out.commands.is_empty());
    }

    #[test]
    #[ignore = "requires WSL2 + openclaw inside WSL"]
    fn wsl2_full_step_chain_runs_on_windows() {
        let artifacts = HashMap::new();
        for step in [
            InstallStep::Precheck,
            InstallStep::Install,
            InstallStep::Init,
            InstallStep::Verify,
        ] {
            let out = wsl2::run_step(&step, &artifacts)
                .unwrap_or_else(|e| panic!("wsl2 step {step:?} failed: {}", e.details));
            assert!(!out.summary.is_empty());
            assert!(!out.commands.is_empty());
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[test]
fn wsl2_tests_are_declared_for_windows_only() {
    assert!(true);
}
