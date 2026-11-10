//! The project's models in Aedifex and Tellus: saved after a change, opened again when the
//! program restarts. Runs against the programs installed on this computer and skips when one
//! is missing.

use std::path::{Path, PathBuf};

use serde_json::json;
use th_harness::{live, Plugins, Settings};

fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("th_live_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn installed(settings: &Settings, name: &str) -> bool {
    settings.plugins.iter().any(|p| p.name == name && p.enabled && Path::new(&p.command).is_file())
}

#[test]
fn aedifex_model_is_saved_and_reopened() {
    let settings = Settings::default();
    if !installed(&settings, "aedifex") {
        eprintln!("Aedifex is not installed here; skipped");
        return;
    }
    let dir = project("aedifex");
    let plugins = Plugins::default();
    let c = plugins.ensure_for(&settings, "aedifex", Some(&dir)).unwrap();
    let r = c.call("add_level", json!({"name": "Roof deck", "elevation": 9.5})).unwrap();
    assert!(!r.is_error, "{}", r.text);
    let m = Plugins::keep_model(&c, "aedifex", "add_level", &dir).expect("the model file is written by --autosave");
    assert!(std::fs::read_to_string(&m).unwrap().contains("Roof deck"));
    assert!(Plugins::keep_model(&c, "aedifex", "get_model", &dir).is_none(), "reading tools do not count as changes");

    // another project restarts the program; coming back opens the saved model again
    let other = project("aedifex_other");
    let c2 = plugins.ensure_for(&settings, "aedifex", Some(&other)).unwrap();
    assert!(!c2.call("get_model", json!({})).unwrap().text.contains("Roof deck"));
    let c3 = plugins.ensure_for(&settings, "aedifex", Some(&dir)).unwrap();
    assert!(c3.call("get_model", json!({})).unwrap().text.contains("Roof deck"));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn tellus_model_is_saved_through_its_tool_and_reopened() {
    let settings = Settings::default();
    if !installed(&settings, "tellus") {
        eprintln!("Tellus is not installed here; skipped");
        return;
    }
    let dir = project("tellus");
    let plugins = Plugins::default();
    let c = plugins.ensure_for(&settings, "tellus", Some(&dir)).unwrap();
    let r = c.call("add_soil_material", json!({"name": "Bangkok soft clay", "model": "mohr_coulomb", "gamma_unsat": 16.0, "gamma_sat": 16.0, "E": 3000.0, "nu": 0.35, "c": 10.0, "phi": 0.0})).unwrap();
    assert!(!r.is_error, "{}", r.text);
    let m = Plugins::keep_model(&c, "tellus", "add_soil_material", &dir).expect("saved with save_project");
    assert_eq!(m, live::model_path(&dir, live::spec("tellus").unwrap()));
    assert!(std::fs::read_to_string(&m).unwrap().contains("Bangkok soft clay"));

    let other = project("tellus_other");
    plugins.ensure_for(&settings, "tellus", Some(&other)).unwrap();
    let c3 = plugins.ensure_for(&settings, "tellus", Some(&dir)).unwrap();
    assert!(c3.call("get_model", json!({})).unwrap().text.contains("Bangkok soft clay"), "opened again with open_project");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}
