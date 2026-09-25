//! A deterministic stand-in for the OpenAI API.
//!
//! - `/v1/embeddings`: a hashed bag-of-words vector per input (single string or the
//!   indexer's batched array, answered with `index`), so texts sharing words are close
//!   and results never depend on a model or network.
//! - `/v1/chat/completions` in JSON mode: the canned plan for the question (planner)
//!   or canned hypotheses (live evidence).
//! - `/v1/chat/completions` otherwise: an answer that cites exactly the documents and
//!   observations the script asks for, numbered by reading the prompt it was given,
//!   like a model following the `[DOC #n]` instructions would.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// Dimension of the fake embeddings.
pub const DIM: usize = 1024;

/// Words too common in questions and logs to say anything about relevance.
const STOPWORDS: &[&str] = &[
    "a",
    "about",
    "after",
    "all",
    "an",
    "and",
    "any",
    "anything",
    "are",
    "as",
    "at",
    "be",
    "by",
    "did",
    "do",
    "does",
    "for",
    "from",
    "has",
    "have",
    "how",
    "in",
    "is",
    "it",
    "its",
    "of",
    "on",
    "or",
    "our",
    "show",
    "that",
    "the",
    "there",
    "this",
    "to",
    "was",
    "we",
    "were",
    "what",
    "when",
    "which",
    "who",
    "why",
    "with",
    "yesterday",
    "today",
];

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Lowercased alphanumeric tokens (Unicode-aware) without stopwords.
pub fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

/// L2-normalized, signed feature-hashed bag of words with sublinear term frequency.
/// The hash sign makes colliding unrelated tokens cancel out on average instead of
/// always adding similarity. A small constant component keeps the vector non-zero
/// for texts without tokens.
pub fn embed(text: &str) -> Vec<f32> {
    let mut counts: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    for t in tokens(text) {
        *counts.entry(t).or_default() += 1.0;
    }
    let mut v = vec![0f32; DIM];
    for (t, n) in counts {
        let h = fnv1a(&t);
        let sign = if h >> 63 == 1 { -1.0 } else { 1.0 };
        v[1 + (h % (DIM as u64 - 1)) as usize] += sign * n.sqrt();
    }
    v[0] = 0.05;
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

/// What the fake answer model cites for one question.
#[derive(Debug, Clone, Default)]
pub struct AnswerScript {
    /// Documents to cite, each as the source URIs it may appear under (a log pattern
    /// is listed under the link of whichever of its days represents it). Each becomes
    /// the `[DOC #n]` of the first prompt document with one of those URIs; documents
    /// missing from the prompt are not cited.
    pub cite_uris: Vec<Vec<String>>,
    /// Observation IDs cited verbatim, e.g. `obs-2`.
    pub cite_observations: Vec<String>,
    /// Appended verbatim (negative controls cite unknown documents here).
    pub extra: String,
}

#[derive(Default)]
pub struct Script {
    pub plans: HashMap<String, Value>,
    pub hypotheses: HashMap<String, Value>,
    pub answers: HashMap<String, AnswerScript>,
    /// The last answer prompt per question.
    pub prompts: HashMap<String, String>,
}

#[derive(Clone, Default)]
pub struct FakeOpenAi {
    pub script: Arc<Mutex<Script>>,
    /// Answer `/v1/embeddings` with 404, for runs whose embeddings come from a real
    /// model (the end-to-end run against text-embeddings-inference).
    pub no_embeddings: bool,
}

fn chat(content: String) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({"choices": [{"message": {"content": content}}]}))
}

/// `[DOC #n]` number of each `Source:` URI in an answer prompt.
pub fn prompt_numbers(prompt: &str) -> Vec<(usize, String, String)> {
    let mut out = vec![];
    let mut current: Option<(usize, String)> = None;
    for line in prompt.lines() {
        if let Some(rest) = line.strip_prefix("[DOC #") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let title = rest[digits.len()..].trim_start_matches("] ").to_string();
            current = digits.parse().ok().map(|n| (n, title));
        } else if let (Some(uri), Some((n, title))) = (line.strip_prefix("Source: "), &current) {
            out.push((*n, title.clone(), uri.to_string()));
            current = None;
        }
    }
    out
}

/// Each `[DOC #n]` block of an answer prompt with its number: the lines from its
/// header up to the next document, the live-evidence timeline or the instructions.
pub fn prompt_blocks(prompt: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = vec![];
    let mut open = false;
    for line in prompt.lines() {
        if let Some(rest) = line.strip_prefix("[DOC #") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            out.push((digits.parse().unwrap_or(0), String::new()));
            open = true;
        } else if line.starts_with("Live evidence timeline") || line == "Instructions:" {
            open = false;
        }
        if open && let Some((_, block)) = out.last_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    out
}

impl FakeOpenAi {
    fn answer(&self, question: &str, prompt: &str) -> String {
        let mut script = self.script.lock().unwrap();
        script
            .prompts
            .insert(question.to_string(), prompt.to_string());
        let Some(spec) = script.answers.get(question) else {
            return "The context does not support a specific answer.".into();
        };
        let numbers = prompt_numbers(prompt);
        let docs: Vec<String> = spec
            .cite_uris
            .iter()
            .filter_map(|uris| numbers.iter().find(|(_, _, u)| uris.contains(u)))
            .map(|(n, _, _)| format!("[DOC #{n}]"))
            .collect();
        let obs: Vec<String> = spec
            .cite_observations
            .iter()
            .map(|o| format!("[{o}]"))
            .collect();
        format!(
            "Answer to: {question}\nEvidence: {}\nObserved: {}\n{}",
            docs.join(" "),
            obs.join(" "),
            spec.extra
        )
    }

    /// The question of a planner request (JSON-mode chat with the planner's system
    /// prompt), whose user message is the question itself.
    pub fn planner_question(req: &Request) -> Option<String> {
        if req.url.path() != "/v1/chat/completions" {
            return None;
        }
        let body: Value = serde_json::from_slice(&req.body).ok()?;
        let system = body["messages"][0]["content"].as_str()?;
        (body.get("response_format").is_some() && system.contains("planning assistant"))
            .then(|| body["messages"][1]["content"].as_str().map(str::to_string))
            .flatten()
    }

    /// The last answer prompt the fake answer model was given for `question`.
    pub fn prompt(&self, question: &str) -> Option<String> {
        self.script.lock().unwrap().prompts.get(question).cloned()
    }

    /// Answers one OpenAI request: `/v1/embeddings` or `/v1/chat/completions`.
    pub fn handle(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        match req.url.path() {
            "/v1/embeddings" if self.no_embeddings => ResponseTemplate::new(404),
            "/v1/embeddings" => {
                let inputs: Vec<String> = match &body["input"] {
                    Value::String(s) => vec![s.clone()],
                    Value::Array(items) => items
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect(),
                    _ => return ResponseTemplate::new(400),
                };
                let data: Vec<Value> = inputs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| json!({"object": "embedding", "index": i, "embedding": embed(t)}))
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({"object": "list", "data": data}))
            }
            "/v1/chat/completions" => {
                let system = body["messages"][0]["content"].as_str().unwrap_or_default();
                let user = body["messages"][1]["content"].as_str().unwrap_or_default();
                let script = self.script.lock().unwrap();
                if body.get("response_format").is_some() {
                    if system.contains("planning assistant") {
                        let plan = script.plans.get(user).cloned().unwrap_or(json!({}));
                        return chat(plan.to_string());
                    }
                    let question = user
                        .strip_prefix("Question: ")
                        .and_then(|r| r.split("\n\n").next())
                        .unwrap_or_default();
                    let h = script
                        .hypotheses
                        .get(question)
                        .cloned()
                        .unwrap_or(json!({"hypotheses": []}));
                    return chat(h.to_string());
                }
                drop(script);
                let question = user
                    .strip_prefix("Question:\n")
                    .and_then(|r| r.split("\n\nContext:").next())
                    .unwrap_or_default();
                chat(self.answer(question, user))
            }
            _ => ResponseTemplate::new(404),
        }
    }

    pub async fn start(&self) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(self.clone())
            .mount(&server)
            .await;
        server
    }
}

impl Respond for FakeOpenAi {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        self.handle(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cos(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn similar_texts_are_closer_than_unrelated_ones() {
        let q = embed("Why was checkout latency high?");
        let related = embed("Checkout p95 latency above 1.5s");
        let unrelated = embed("Kortbetalningar misslyckas 💳");
        assert!(cos(&q, &related) > 0.4);
        assert!(cos(&q, &related) > 3.0 * cos(&q, &unrelated));
        assert_eq!(embed("Åsa"), embed("åsa"));
        assert!((cos(&q, &q) - 1.0).abs() < 1e-5);
        assert!(embed("?!").iter().any(|x| *x > 0.0));
    }

    #[test]
    fn prompt_numbers_pair_each_doc_with_its_source() {
        let prompt = "Context:\n[DOC #1] Checkout (Monitor)\nTime: x\nSource: https://a/1\nScore: 1\n\
                      [DOC #2] Betalningar – översikt 📊 (Dashboard)\nSource: https://a/2\n";
        assert_eq!(
            prompt_numbers(prompt),
            vec![
                (1, "Checkout (Monitor)".into(), "https://a/1".into()),
                (
                    2,
                    "Betalningar – översikt 📊 (Dashboard)".into(),
                    "https://a/2".into()
                )
            ]
        );
    }
}
