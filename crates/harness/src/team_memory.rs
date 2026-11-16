//! Team memory (option `harness.team_memory`): a searchable memory shared by every agent of
//! a team, holding only grounded records, so that a specialist works from the engineer's own
//! words and from tool results instead of from the coordinator's paraphrase.
//!
//! The records are the engineer's messages, verbatim, and every fact set in force in the
//! root session (the tools' pinned results, including those promoted from delegated
//! agents). Agents' own claims are left out on purpose, so one agent's mistake cannot become
//! another's memory. Two things use it:
//!
//! * every delegated task is followed by a briefing with the engineer's request of this
//!   turn, word for word, and the records most relevant to the task;
//! * the `team_memory` tool lets any agent search the records itself.
//!
//! Records are ranked by the cosine of their embeddings (the embedder of the standards
//! library: a learned model through an API when one is set, otherwise the offline hashed
//! n-gram vectors) plus a small bonus for shared terms.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use th_hive::facts;
use th_hive::{keys, Agent, AgentRegistry, Context, EventBody, Session, ToolDef, ToolOutput};
use th_kb::EmbedderConfig;

use crate::project::clip;

/// One grounded record.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Where the record comes from, as the agent is told.
    pub source: String,
    pub text: String,
}

/// The agent at the top of `a`'s delegation tree (whose session is the conversation).
pub fn root_agent(ctx: &Context, a: &Arc<Agent>) -> Arc<Agent> {
    let Some(reg) = ctx.get::<AgentRegistry>(keys::AGENTS) else { return a.clone() };
    let mut cur = a.clone();
    while let Some(p) = cur.parent.as_ref().and_then(|p| reg.get(p)) {
        cur = p;
    }
    cur
}

/// The engineer's latest message in the session.
pub fn latest_request(s: &Session) -> Option<String> {
    s.events().into_iter().rev().find_map(|e| match e.body {
        EventBody::UserMessage { text, source, .. } if source == "user" => Some(text),
        _ => None,
    })
}

fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    for para in text.split("\n\n") {
        if !cur.is_empty() && cur.len() + para.len() > max {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(para);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// The grounded records of a conversation: the engineer's words and the pinned results in force.
pub fn entries(root: &Session) -> Vec<Entry> {
    let mut out = vec![];
    let mut turn = 0;
    for e in root.events() {
        match e.body {
            EventBody::TurnStart { turn: t } => turn = t,
            EventBody::UserMessage { text, source, .. } if source == "user" => {
                for c in chunks(&text, 1200) {
                    out.push(Entry { source: format!("the engineer's own words, turn {turn}"), text: c });
                }
            }
            _ => {}
        }
    }
    let records = root.fact_records();
    for r in facts::current(&records) {
        let origin = if r.set.origin.is_empty() { r.set.group.clone() } else { r.set.origin.clone() };
        out.push(Entry { source: format!("a tool result pinned in the log: {origin}, from {}", r.tool), text: facts::render(r) });
    }
    out
}

fn cache() -> &'static Mutex<HashMap<String, Vec<f32>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<f32>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    th_kb::embed::normalize(&mut v);
    v
}

/// Vectors of `texts` under `cfg`. An API embedder caches each text's vector; when it fails,
/// the offline hashed vectors are used, so search never stops the agent.
fn embed(cfg: &EmbedderConfig, texts: &[String]) -> (Vec<Vec<f32>>, &'static str) {
    if let EmbedderConfig::Api { base_url, api_key, model } = cfg {
        let missing: Vec<String> = {
            let c: Vec<String> = cache().lock().map(|c| texts.iter().filter(|t| !c.contains_key(*t)).cloned().collect()).unwrap_or_default();
            let mut seen = std::collections::HashSet::new();
            c.into_iter().filter(|t| seen.insert(t.clone())).collect()
        };
        let fetched = if missing.is_empty() { Ok(vec![]) } else { th_kb::embed::api_embed(base_url, api_key.as_deref(), model, &missing, &mut |_, _| {}) };
        if let Ok(vs) = fetched {
            if let Ok(mut c) = cache().lock() {
                for (t, v) in missing.into_iter().zip(vs) {
                    c.insert(t, normalized(v));
                }
                if texts.iter().all(|t| c.contains_key(t)) {
                    return (texts.iter().map(|t| c[t].clone()).collect(), "api");
                }
            }
        }
    }
    let idf = th_kb::embed::HashedIdf::build(texts.iter().map(|s| s.as_str()));
    (texts.iter().map(|t| th_kb::embed::hashed_embed(t, &idf, 1024)).collect(), "hashed")
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn overlap(a: &str, b: &str) -> f32 {
    let ta: std::collections::HashSet<String> = th_kb::text::terms(a).into_iter().collect();
    let tb: std::collections::HashSet<String> = th_kb::text::terms(b).into_iter().collect();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    ta.intersection(&tb).count() as f32 / ta.len().min(tb.len()) as f32
}

/// The `k` records most relevant to `query`, best first, with their scores.
pub fn search(cfg: &EmbedderConfig, query: &str, records: &[Entry], k: usize) -> (Vec<(f32, Entry)>, &'static str) {
    if records.is_empty() || k == 0 {
        return (vec![], "none");
    }
    let mut texts: Vec<String> = records.iter().map(|e| e.text.clone()).collect();
    texts.push(query.to_string());
    let (vs, how) = embed(cfg, &texts);
    let q = &vs[vs.len() - 1];
    let mut scored: Vec<(f32, Entry)> = records.iter().zip(&vs).map(|(e, v)| (cosine(q, v) + 0.2 * overlap(query, &e.text), e.clone())).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    (scored, how)
}

fn render(hits: &[(f32, Entry)]) -> String {
    hits.iter().enumerate().map(|(i, (_, e))| format!("[{}] {}\n{}", i + 1, e.source, clip(&e.text, 1500))).collect::<Vec<_>>().join("\n\n")
}

/// The briefing that follows a delegated task: the engineer's request of this turn, word for
/// word, and the grounded records most relevant to the task. Empty when there is no request.
pub fn briefing(ctx: &Context, parent: &Arc<Agent>, task: &str, cfg: &EmbedderConfig) -> String {
    let root = root_agent(ctx, parent);
    let Some(request) = latest_request(&root.session) else { return String::new() };
    let latest: Vec<String> = chunks(&request, 1200);
    let records: Vec<Entry> = entries(&root.session).into_iter().filter(|e| !latest.contains(&e.text)).collect();
    let (hits, _) = search(cfg, task, &records, 6);
    let mut out = format!(
        "\n\n---\nTEAM MEMORY: verbatim records from the session log, not the coordinator's paraphrase.\n\nThe engineer's request in this turn, word for word:\n<<<\n{}\n>>>",
        clip(&request, 8000)
    );
    if !hits.is_empty() {
        out.push_str(&format!("\n\nRecords most relevant to this task (the engineer's earlier words and pinned tool results):\n{}", render(&hits)));
    }
    out.push_str("\n\nWhere the task above differs from these records, the records are right. Search for more with the team_memory tool.");
    out
}

/// The `team_memory` search tool; `enabled` decides per call whether agents see it.
pub fn tool(embedder: impl Fn() -> EmbedderConfig + Send + Sync + 'static, enabled: impl Fn() -> bool + Send + Sync + 'static) -> ToolDef {
    ToolDef::new(
        "team_memory",
        "Search the team's shared memory: the engineer's own words and every result a tool returned in this conversation (pinned values, with the calculation they come from), across all agents. Use it to read the real question and the real numbers before you rely on a task's wording or on your recall.",
        json!({"type":"object","properties":{"query":{"type":"string","description":"What you need, e.g. 'which method did the engineer ask for' or 'q_ult of case8'"},"k":{"type":"integer","description":"Number of records (default 5)"}},"required":["query"]}),
        move |t, a: Value| {
            let q = a.get("query").and_then(|v| v.as_str()).ok_or("missing 'query'")?;
            let k = a.get("k").and_then(|v| v.as_u64()).unwrap_or(5).clamp(1, 20) as usize;
            let root = root_agent(&t.ctx, &t.agent);
            let records = entries(&root.session);
            let (hits, how) = search(&embedder(), q, &records, k);
            if hits.is_empty() {
                return Ok(ToolOutput::ok("The team memory holds no records yet."));
            }
            Ok(ToolOutput::ok(format!("{} of {} records ({} search):\n\n{}", hits.len(), records.len(), how, render(&hits))))
        },
    )
    .concurrent()
    .always()
    .when(move |_| enabled())
}

#[cfg(test)]
mod tests {
    use super::*;
    use th_hive::{Emit, Fact, FactRecord, FactSet, SessionMeta};

    fn session() -> Arc<Session> {
        let s = Session::memory(SessionMeta { id: "r".into(), agent: "coordinator".into(), ..Default::default() }, Arc::new(Emit::new("session/event")));
        s.append(EventBody::TurnStart { turn: 1 });
        s.append(EventBody::UserMessage { text: "Parametric study of a square pad footing by Terzaghi (1943). Use the built-in bearing_terzaghi design module.\n\nCases: case8 B = 1.5 m, phi = 40 deg.".into(), images: vec![], source: "user".into(), surface_op: None });
        s.append(EventBody::UserMessage { text: "Task from coordinator: check it".into(), images: vec![], source: "parent".into(), surface_op: None });
        let set = FactSet { group: "calcs/case8".into(), origin: "calcs/case8.json".into(), headline: "Terzaghi".into(), facts: vec![Fact::new("result", "N_g", "115.3", ""), Fact::new("result", "q_ult", "2708", "kPa").key()] };
        s.append(EventBody::Custom { kind: facts::KIND.into(), data: serde_json::to_value(FactRecord { source: 5, tool: "delegate".into(), set }).unwrap() });
        s.append(EventBody::TurnStart { turn: 2 });
        s.append(EventBody::UserMessage { text: "Now check six other footings S1 to R2.".into(), images: vec![], source: "user".into(), surface_op: None });
        s
    }

    #[test]
    fn records_are_the_engineers_words_and_pinned_results_only() {
        let s = session();
        let es = entries(&s);
        assert_eq!(es.len(), 3, "{es:?}");
        assert!(es.iter().all(|e| !e.text.contains("Task from coordinator")), "agents' words are not records");
        assert!(es[0].source.contains("turn 1") && es[0].text.contains("bearing_terzaghi"));
        assert!(es[1].source.contains("turn 2"));
        assert!(es[2].source.contains("calcs/case8.json") && es[2].text.contains("q_ult = 2708 kPa"));
        assert_eq!(latest_request(&s).as_deref(), Some("Now check six other footings S1 to R2."));
    }

    #[test]
    fn search_finds_the_method_and_the_value_offline() {
        let es = entries(&session());
        let cfg = EmbedderConfig::Hashed { dim: 1024 };
        let (hits, how) = search(&cfg, "which design module did the engineer ask for", &es, 1);
        assert_eq!(how, "hashed");
        assert!(hits[0].1.text.contains("bearing_terzaghi"), "{hits:?}");
        let (hits, _) = search(&cfg, "q_ult of case8 at phi 40", &es, 1);
        assert!(hits[0].1.text.contains("2708"), "{hits:?}");
    }
}
