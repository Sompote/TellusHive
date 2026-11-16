//! Standards briefing: before an agent starts a task, the standards library is searched with
//! the task's own words and the passages that really match it are put in front of the agent,
//! with their source, clause and page. Agents that never call `search_standards` (most of them,
//! in the GeoTechBench runs) then still see the method the task names, as the library states it.

use th_kb::{Hit, Library};

/// Passages shown at most.
const MAX_HITS: usize = 3;
/// Characters of passage text shown at most, over all passages.
const MAX_CHARS: usize = 5_000;
/// Characters of the task used as one query; the embedding of a very long text is blurred.
const QUERY_CHARS: usize = 1_200;

/// Words that mark the sentence naming the method or the standard the task asks for, matched at
/// the start of a word (so "use" does not match "because").
const METHOD_MARKS: [&str; 12] = ["method", "according to", "using ", "use ", "apply ", "applying ", "by the ", "per ", "following ", "after ", "procedure", "approach"];
/// Queries searched at most for one task.
const MAX_QUERIES: usize = 6;

/// The queries searched for a task. The first is the task text (clipped), used only for the
/// clauses, tables and figures it names: a long text scores 0.65 to 0.76 against most chunks of
/// the library, above the threshold calibrated on short queries, so its meaning tells little.
/// The others are short enough that the 0.58 similarity threshold holds: the task's opening
/// (which often states what is wanted) and each sentence that names a method or standard. The
/// opening is always searched, because a sentence about the answer format ("one value per
/// quantity") can pass as a method sentence while the method itself is phrased otherwise.
pub fn queries(task: &str) -> Vec<String> {
    let task = task.trim();
    let mut qs = vec![clip_chars(task, QUERY_CHARS)];
    if task.len() > 300 {
        qs.push(clip_chars(task, 300));
    }
    for s in task.split(['.', '?', '!', '\n']) {
        if qs.len() >= MAX_QUERIES {
            break;
        }
        let s = s.trim();
        let low = format!(" {}", s.to_lowercase());
        if s.len() >= 12 && METHOD_MARKS.iter().any(|m| low.contains(&format!(" {m}"))) {
            let q = clip_chars(s, 300);
            if !qs.contains(&q) {
                qs.push(q);
            }
        }
    }
    qs.retain(|q| !q.is_empty());
    qs
}

/// The relevant passages for a task, best first, at most `MAX_HITS`, from the given standards
/// (all standards when the list is empty).
pub fn relevant_hits(library: &Library, task: &str, standard_ids: &[String]) -> Vec<Hit> {
    let mut best: Vec<Hit> = vec![];
    for (i, q) in queries(task).into_iter().enumerate() {
        let Ok(hits) = library.search(&q, standard_ids, 6) else { continue };
        let whole = i == 0 && q.len() > 300;
        for h in hits.into_iter().filter(|h| if whole { h.clause_match } else { h.is_relevant() }) {
            let key = |x: &Hit| (x.standard_id.clone(), x.clause.clone(), x.page);
            match best.iter_mut().find(|b| key(b) == key(&h)) {
                Some(b) => {
                    if rank(&h) > rank(b) {
                        *b = h;
                    }
                }
                None => best.push(h),
            }
        }
    }
    best.sort_by(|a, b| rank(b).partial_cmp(&rank(a)).unwrap_or(std::cmp::Ordering::Equal));
    best.truncate(MAX_HITS);
    best
}

/// How strongly a relevant hit matches: a named clause first, then meaning, then shared terms.
fn rank(h: &Hit) -> f32 {
    (if h.clause_match { 10.0 } else { 0.0 }) + h.similarity.unwrap_or(0.0) + 0.1 * h.coverage
}

/// The briefing text for a task, or an empty string when no passage matches it closely.
pub fn briefing(library: &Library, task: &str, standard_ids: &[String]) -> String {
    render(&relevant_hits(library, task, standard_ids))
}

/// The briefing text for the given passages.
pub fn render(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\n\n[Standards briefing: passages from the project's standards library that match this task closely. \
         When the task names a method, use its equations as written here and cite the standard, clause and page. \
         If a passage does not fit the task, say so and state the source you use instead.]\n",
    );
    let mut left = MAX_CHARS;
    for h in hits {
        if left < 200 {
            break;
        }
        let page = h.page.map(|p| format!(", page {p}")).unwrap_or_default();
        let clause = if h.clause.is_empty() { String::new() } else { format!(", clause {}", h.clause) };
        let text = clip_chars(h.text.trim(), left);
        left = left.saturating_sub(text.len());
        s.push_str(&format!("\n--- {}{clause}{page}: {}\n{text}\n", h.standard_code, h.title));
    }
    s
}

fn clip_chars(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} …", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(clause: &str, sim: f32) -> Hit {
        Hit { standard_id: "s".into(), standard_code: "TMN-1".into(), clause: clause.into(), title: "Pile group efficiency".into(), page: Some(2), text: "E = 1 - theta (n-1)m + (m-1)n / (90 m n)".into(), score: 0.03, similarity: Some(sim), coverage: 0.5, clause_match: false }
    }

    #[test]
    fn queries_keep_the_method_sentence() {
        let q = queries("A 3 x 4 pile group has piles at 1.2 m spacing. Compute the group efficiency using the Converse-Labarre method. Give E.");
        assert_eq!(q.len(), 2);
        assert!(q[1].contains("Converse-Labarre"));
        // "use" marks a method sentence, but not inside "because"
        let q = queries("The wall moves because the soil yields. Use the Purdue method for alpha.");
        assert_eq!(q.len(), 2);
        assert!(q[1].starts_with("Use the Purdue"));
    }

    #[test]
    fn render_cites_source_and_is_empty_without_hits() {
        assert_eq!(render(&[]), "");
        let s = render(&[hit("2", 0.66)]);
        assert!(s.contains("TMN-1, clause 2, page 2"));
        assert!(s.contains("cite the standard, clause and page"));
    }
}
