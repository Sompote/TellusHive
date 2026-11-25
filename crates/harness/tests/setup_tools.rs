//! The project set-up tools a bot uses from the chat: they change the project's standards and
//! plugins, save project.json, and the next tool call sees the change.

use std::sync::Arc;

use serde_json::json;
use th_harness::{config, Engine, Plugins, Project, ProjectInfo, Settings};

#[test]
fn set_project_standards_and_plugins() {
    let tmp = std::env::temp_dir().join(format!("th_setup_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let library = Arc::new(th_kb::Library::open(&tmp.join("lib")).unwrap());
    let src = tmp.join("aci.md");
    std::fs::write(&src, "# 22.5 One-way shear\n\n22.5.5.1 Vc = 0.17 lambda sqrt(fc) bw d.\n").unwrap();
    let meta = th_kb::NewStandard { name: "Building code".into(), code: "ACI 318-19".into(), discipline: "structural".into(), country: "US".into(), year: Some(2019) };
    library.ingest(&[src], &meta, &th_kb::EmbedderConfig::default(), &mut |_, _| {}).unwrap();
    let project = Project::create(&tmp.join("p"), ProjectInfo { name: "P".into(), ..Default::default() }).unwrap();
    let settings = Settings::default();
    let names: Vec<String> = settings.plugins.iter().filter(|p| p.enabled).map(|p| p.name.clone()).collect();
    let engine = Engine::new(settings, config::default_team(), project.clone(), library, Arc::new(Plugins::default()));
    let d = &engine.domain;

    let r = d.setup_tool("set_project_standards", &json!({"add": ["aci 318-19"]})).unwrap();
    assert!(r.contains("ACI 318-19"), "{r}");
    let saved = Project::open(&project.dir).unwrap();
    assert_eq!(saved.info.standards.len(), 1);
    assert_eq!(d.project().info.standards, saved.info.standards);

    let e = d.setup_tool("set_project_standards", &json!({"add": ["EN 1997-1"]})).unwrap_err();
    assert!(e.contains("not in the standards library") && e.contains("ACI 318-19"), "{e}");
    assert_eq!(Project::open(&project.dir).unwrap().info.standards.len(), 1, "a bad code changes nothing");

    if let Some(first) = names.first() {
        d.setup_tool("set_project_plugins", &json!({"remove": [first]})).unwrap();
        let info = Project::open(&project.dir).unwrap().info;
        assert!(!info.allows_plugin(first));
        assert_eq!(info.plugins.as_ref().unwrap().len(), names.len() - 1);
        let l = d.setup_tool("list_plugins", &json!({})).unwrap();
        assert!(l.contains(&format!("  {first}:")), "{l}");
        d.setup_tool("set_project_plugins", &json!({"all": true})).unwrap();
        assert!(Project::open(&project.dir).unwrap().info.plugins.is_none());
    }
    assert!(d.setup_tool("set_project_plugins", &json!({"add": ["nope"]})).unwrap_err().contains("no plugin named nope"));
    let _ = std::fs::remove_dir_all(&tmp);
}
