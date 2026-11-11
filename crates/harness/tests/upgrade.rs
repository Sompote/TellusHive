//! A team and settings saved by an earlier version take up the current built-in texts when
//! they still hold an earlier default, and keep what the engineer wrote.

use th_harness::config::{default_team, OLD_AGENT_TEXTS, OLD_PLUGIN_DESCRIPTIONS};
use th_harness::{Settings, TeamSpec};

#[test]
fn old_defaults_are_refreshed_and_edits_kept() {
    let home = std::env::temp_dir().join(format!("th_upgrade_{}", std::process::id()));
    std::fs::create_dir_all(home.join("teams")).unwrap();
    // a separate test binary, so setting the variable affects no other test
    std::env::set_var("TELLUSHIVE_HOME", &home);

    let current = default_team();
    let mut saved = current.clone();
    // the BIM modeller still as the interim version wrote it (told to refuse a slope)
    let old_bim = OLD_AGENT_TEXTS.iter().filter(|(id, f, _)| *id == "bim_modeller" && *f == "instructions").last().unwrap().2;
    let old_role = OLD_AGENT_TEXTS.iter().find(|(id, f, _)| *id == "bim_modeller" && *f == "role").unwrap().2;
    let bim = saved.agents.iter_mut().find(|a| a.id == "bim_modeller").unwrap();
    bim.instructions = old_bim.to_string();
    bim.role = old_role.to_string();
    // the coordinator with a sentence the engineer added
    let coord = saved.agents.iter_mut().find(|a| a.id == "coordinator").unwrap();
    coord.instructions.push_str(" First start with the BIM model.");
    let edited = coord.instructions.clone();
    saved.save().unwrap();

    let t = TeamSpec::load(&saved.name);
    let get = |id: &str| t.agents.iter().find(|a| a.id == id).unwrap();
    let want = |id: &str| current.agents.iter().find(|a| a.id == id).unwrap();
    assert_eq!(get("bim_modeller").instructions, want("bim_modeller").instructions);
    assert_eq!(get("bim_modeller").role, want("bim_modeller").role);
    assert!(get("bim_modeller").instructions.contains("add_graded_slope"));
    assert_eq!(get("coordinator").instructions, edited, "an edited text is kept");

    // settings: the old Aedifex description follows the program, an edited one stays
    let old_desc = OLD_PLUGIN_DESCRIPTIONS.iter().find(|(n, _)| *n == "aedifex").unwrap().1;
    let mut s = Settings::load();
    s.plugins.iter_mut().find(|p| p.name == "aedifex").unwrap().description = old_desc.to_string();
    s.plugins.iter_mut().find(|p| p.name == "pondus").unwrap().description = "my Pondus".into();
    s.save().unwrap();
    let s = Settings::load();
    assert!(s.plugins.iter().find(|p| p.name == "aedifex").unwrap().description.contains("terrain"));
    assert_eq!(s.plugins.iter().find(|p| p.name == "pondus").unwrap().description, "my Pondus");

    std::fs::remove_dir_all(&home).ok();
}
