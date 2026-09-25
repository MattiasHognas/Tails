//! Sparse keyword vectors for hybrid search.
//!
//! Dense embeddings are weak at the exact tokens incident questions quote: error codes
//! (`ERR_CONN_RESET`), exception class names, metric names
//! (`trace.http.request.errors`), hostnames and IDs. Every chunk is therefore also
//! stored with a sparse BM25-style vector over its [embedding
//! input](crate::chunk::embedding_input), and questions are searched with both (see
//! [`crate::qdrant::Qdrant::hybrid_search`]).
//!
//! - **Tokens** ([`tokenize`]): runs of letters and digits (Unicode-aware, combining
//!   marks stay with their base), lowercased. An identifier joined by `_`, `-` or `.`
//!   is kept whole *and* split into its parts, and a camelCase or PascalCase part also
//!   yields its words, so `ERR_CONN_RESET` matches exactly while `conn` or `reset`
//!   still match partially. `:` and `/` join coarser units: `service:checkout`,
//!   `api/v1/login` and the `avg:trace.http.request.errors` of a Datadog query are
//!   kept whole, and each segment is tokenized as above, so the metric name inside a
//!   monitor query is a token too. Joiners at the ends of a run (sentence dots,
//!   `service:`) are dropped.
//! - **Indices** ([`token_index`]): the 32-bit FNV-1a hash of the lowercased token's
//!   UTF-8 bytes. It is stable across runs, platforms and Rust versions; indices of
//!   stored points must never change for the same text.
//! - **Document values** ([`document_vector`]): the BM25 term-frequency part,
//!   `tf·(k1+1) / (tf + k1·(1 − b + b·len/avg_len))` with `k1` = 1.2, `b` = 0.75 and a
//!   fixed `avg_len` of [`AVG_DOC_TOKENS`], so repeating a word saturates and long
//!   chunks do not win by length alone.
//! - **Query values** ([`query_vector`]): 1 per distinct token.
//! - **IDF** is applied by Qdrant (`modifier: idf` on the sparse vector), from the
//!   whole collection: `ln(1 + (N − n + 0.5) / (n + 0.5))`, N points with a sparse
//!   vector, n of them containing the token. The score of a point is
//!   `Σ query value · idf · document value` over shared indices: BM25 with a fixed
//!   average length.
//!
//! Changing anything here changes stored vectors: bump [`SPARSE_ENCODER_VERSION`], which
//! is part of every content hash, so the indexer rewrites every document.

use crate::text::is_extender;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Version of the tokenizer and weighting; part of [`crate::chunk::content_hash`].
pub const SPARSE_ENCODER_VERSION: &str = "tails-bm25-v1";

/// BM25 term-frequency saturation.
const K1: f32 = 1.2;
/// BM25 length normalization.
const B: f32 = 0.75;
/// Assumed average document length in tokens (an 1800-char chunk plus its header is
/// roughly 250–350 tokens; log patterns and catalog entries are much shorter).
pub const AVG_DOC_TOKENS: f32 = 256.0;

/// A sparse vector as Qdrant takes it: unique `indices`, ascending, and their `values`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SparseVector {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

impl SparseVector {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn from_map(map: BTreeMap<u32, f32>) -> Self {
        let (indices, values) = map.into_iter().unzip();
        Self { indices, values }
    }
}

/// Characters that join the parts of an identifier: `ERR_CONN_RESET`, `auth-api`,
/// `trace.http.request.errors`.
fn is_inner_joiner(c: char) -> bool {
    matches!(c, '_' | '-' | '.')
}

/// Characters that join identifiers into larger units: `service:checkout`,
/// `api/v1/login`, `avg:trace.http.request.errors`.
fn is_outer_joiner(c: char) -> bool {
    matches!(c, '/' | ':')
}

fn is_joiner(c: char) -> bool {
    is_inner_joiner(c) || is_outer_joiner(c)
}

/// Lowercased tokens of `text`, in order and with repeats (see the module docs).
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        // A combining mark or other extender belongs to the letter before it.
        let attached = is_extender(c) && run.chars().next_back().is_some_and(|p| !is_joiner(p));
        if c.is_alphanumeric() || is_joiner(c) || attached {
            run.push(c);
        } else {
            emit_run(&run, &mut out);
            run.clear();
        }
    }
    emit_run(&run, &mut out);
    out
}

/// Tokens of one run of word characters and joiners: the whole run when it has
/// several segments, then each segment's tokens.
fn emit_run(run: &str, out: &mut Vec<String>) {
    let run = run.trim_matches(is_joiner);
    let segments: Vec<&str> = run
        .split(is_outer_joiner)
        .map(|s| s.trim_matches(is_inner_joiner))
        .filter(|s| !s.is_empty())
        .collect();
    if segments.len() > 1 {
        out.push(run.to_lowercase());
    }
    for segment in segments {
        let parts: Vec<&str> = segment
            .split(is_inner_joiner)
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() > 1 {
            out.push(segment.to_lowercase());
        }
        for part in parts {
            out.push(part.to_lowercase());
            let words = case_words(part);
            if words.len() > 1 {
                out.extend(words.iter().map(|w| w.to_lowercase()));
            }
        }
    }
}

/// The words of a camelCase or PascalCase part: `SocketTimeoutException` → `Socket`,
/// `Timeout`, `Exception`; `HTTPServer` → `HTTP`, `Server`. Other parts are one word.
fn case_words(part: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = part.char_indices().collect();
    let mut cuts = vec![0];
    for i in 1..chars.len() {
        let (pos, c) = chars[i];
        let prev = chars[i - 1].1;
        let next_lower = chars.get(i + 1).is_some_and(|(_, n)| n.is_lowercase());
        if c.is_uppercase()
            && (prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower))
        {
            cuts.push(pos);
        }
    }
    cuts.push(part.len());
    cuts.windows(2).map(|w| &part[w[0]..w[1]]).collect()
}

/// Stable index of a token: 32-bit FNV-1a of its UTF-8 bytes.
pub fn token_index(token: &str) -> u32 {
    token.bytes().fold(0x811c_9dc5, |h: u32, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// How often each token index occurs in `text` (hash collisions add up).
pub fn term_frequencies(text: &str) -> BTreeMap<u32, u32> {
    let mut tf = BTreeMap::new();
    for t in tokenize(text) {
        *tf.entry(token_index(&t)).or_default() += 1;
    }
    tf
}

/// The stored sparse vector of a chunk's embedding input: BM25 term-frequency weights.
pub fn document_vector(text: &str) -> SparseVector {
    let tf = term_frequencies(text);
    let len: u32 = tf.values().sum();
    let norm = K1 * (1.0 - B + B * len as f32 / AVG_DOC_TOKENS);
    SparseVector::from_map(
        tf.into_iter()
            .map(|(i, n)| {
                let n = n as f32;
                (i, n * (K1 + 1.0) / (n + norm))
            })
            .collect(),
    )
}

/// The sparse query vector of a question: 1 per distinct token. Empty when the
/// question has no tokens; the caller then skips the keyword search.
pub fn query_vector(text: &str) -> SparseVector {
    let distinct: BTreeSet<String> = tokenize(text).into_iter().collect();
    let mut map = BTreeMap::new();
    for t in distinct {
        *map.entry(token_index(&t)).or_insert(0.0) += 1.0;
    }
    SparseVector::from_map(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(text: &str) -> Vec<String> {
        tokenize(text)
    }

    #[test]
    fn identifiers_are_kept_whole_and_split_into_parts() {
        assert_eq!(
            toks("upstream ERR_CONN_RESET."),
            ["upstream", "err_conn_reset", "err", "conn", "reset"]
        );
        assert_eq!(toks("auth-api"), ["auth-api", "auth", "api"]);
        assert_eq!(
            toks("trace.http.request.errors"),
            [
                "trace.http.request.errors",
                "trace",
                "http",
                "request",
                "errors"
            ]
        );
        assert_eq!(
            toks("ip-10-0-1-23.eu-west-1.compute.internal"),
            [
                "ip-10-0-1-23.eu-west-1.compute.internal",
                "ip",
                "10",
                "0",
                "1",
                "23",
                "eu",
                "west",
                "1",
                "compute",
                "internal"
            ]
        );
        // Tags and paths are identifiers too.
        assert_eq!(
            toks("service:checkout GET /api/v1/login"),
            [
                "service:checkout",
                "service",
                "checkout",
                "get",
                "api/v1/login",
                "api",
                "v1",
                "login"
            ]
        );
    }

    #[test]
    fn metric_names_in_datadog_queries_are_tokens() {
        assert_eq!(
            toks(
                "sum(last_5m):sum:trace.http.request.errors{service:checkout,env:prod}.as_count() > 50"
            ),
            [
                "sum",
                "last_5m",
                "last",
                "5m",
                "sum:trace.http.request.errors",
                "sum",
                "trace.http.request.errors",
                "trace",
                "http",
                "request",
                "errors",
                "service:checkout",
                "service",
                "checkout",
                "env:prod",
                "env",
                "prod",
                "as_count",
                "as",
                "count",
                "50"
            ]
        );
        assert_eq!(
            toks("https://app.datadoghq.eu/logs"),
            [
                "https://app.datadoghq.eu/logs",
                "https",
                "app.datadoghq.eu",
                "app",
                "datadoghq",
                "eu",
                "logs"
            ]
        );
    }

    #[test]
    fn exception_class_names_yield_their_words() {
        assert_eq!(
            toks("java.net.SocketTimeoutException: Read timed out"),
            [
                "java.net.sockettimeoutexception",
                "java",
                "net",
                "sockettimeoutexception",
                "socket",
                "timeout",
                "exception",
                "read",
                "timed",
                "out"
            ]
        );
        assert_eq!(toks("HTTPServer"), ["httpserver", "http", "server"]);
        assert_eq!(toks("Http2Client"), ["http2client", "http2", "client"]);
        // All caps and single words are not split.
        assert_eq!(toks("ERR Timeout"), ["err", "timeout"]);
    }

    #[test]
    fn punctuation_and_edge_joiners_separate_tokens() {
        assert_eq!(
            toks("pool exhausted: 50/50 (checkout → payments), retry..."),
            [
                "pool",
                "exhausted",
                "50/50",
                "50",
                "50",
                "checkout",
                "payments",
                "retry"
            ]
        );
        assert_eq!(toks("--flag_ -x- a__b"), ["flag", "x", "a__b", "a", "b"]);
    }

    #[test]
    fn unicode_text_is_lowercased_and_never_split_inside_a_character() {
        assert_eq!(
            toks("Återförsök för ÅSA.Öberg"),
            ["återförsök", "för", "åsa.öberg", "åsa", "öberg"]
        );
        // A decomposed "é" (e + U+0301) stays one word; emoji and ZWJ sequences separate.
        assert_eq!(
            toks("cafe\u{0301} 👩\u{200D}💻deploy🚀failed"),
            ["cafe\u{0301}", "deploy", "failed"]
        );
        assert_eq!(
            toks("決済ゲートウェイ タイムアウト"),
            ["決済ゲートウェイ", "タイムアウト"]
        );
        // Uppercase non-ASCII letters split camel case like ASCII.
        assert_eq!(toks("ÖrebroFel"), ["örebrofel", "örebro", "fel"]);
    }

    #[test]
    fn empty_and_punctuation_only_text_has_no_tokens() {
        assert!(toks("").is_empty());
        assert!(toks("  -- ... :: → 🚀 ").is_empty());
        assert!(query_vector("?!").is_empty());
        assert!(document_vector("").is_empty());
    }

    #[test]
    fn token_indices_are_stable_fnv1a() {
        // Pinned: stored points depend on these values.
        assert_eq!(token_index(""), 0x811c_9dc5);
        assert_eq!(token_index("a"), 0xe40c_292c);
        assert_eq!(token_index("err_conn_reset"), token_index("err_conn_reset"));
        assert_ne!(
            token_index("err_conn_reset"),
            token_index("err_conn_refused")
        );
        assert_eq!(
            query_vector("ERR_CONN_RESET").indices,
            query_vector("err_conn_reset").indices
        );
    }

    #[test]
    fn document_vectors_hold_saturated_term_frequencies() {
        let text = "auth-api auth login";
        let tf = term_frequencies(text);
        // auth-api, auth (twice), api, login: 5 tokens, 4 distinct.
        assert_eq!(tf.values().sum::<u32>(), 5);
        assert_eq!(tf[&token_index("auth")], 2);
        assert_eq!(tf[&token_index("auth-api")], 1);
        let v = document_vector(text);
        assert_eq!(v.indices.len(), 4);
        assert!(v.indices.windows(2).all(|w| w[0] < w[1]), "ascending");
        let norm = K1 * (1.0 - B + B * 5.0 / AVG_DOC_TOKENS);
        let weight = |i: u32| v.values[v.indices.iter().position(|x| *x == i).unwrap()];
        assert!((weight(token_index("auth")) - 2.0 * 2.2 / (2.0 + norm)).abs() < 1e-6);
        assert!((weight(token_index("login")) - 2.2 / (1.0 + norm)).abs() < 1e-6);
        // Repeats saturate: ten times the word is worth far less than ten times.
        let once = document_vector("reset").values[0];
        let ten = document_vector(&"reset ".repeat(10)).values[0];
        assert!(ten > once && ten < 2.0 * once, "{once} {ten}");
        // The same text always gives the same vector.
        assert_eq!(document_vector(text), document_vector(text));
    }

    #[test]
    fn query_vectors_count_each_distinct_token_once() {
        let q = query_vector("ERR_CONN_RESET err err");
        assert_eq!(q.indices.len(), 4);
        assert!(q.values.iter().all(|v| *v == 1.0));
        let mut want: Vec<u32> = ["err_conn_reset", "err", "conn", "reset"]
            .iter()
            .map(|t| token_index(t))
            .collect();
        want.sort_unstable();
        assert_eq!(q.indices, want);
    }

    #[test]
    fn sparse_vectors_serialize_as_qdrant_expects() {
        let v = SparseVector {
            indices: vec![1, 7],
            values: vec![0.5, 2.0],
        };
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            serde_json::json!({"indices": [1, 7], "values": [0.5, 2.0]})
        );
    }
}
