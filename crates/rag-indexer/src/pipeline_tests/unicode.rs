//! Multibyte text through the whole pipeline.
//!
//! Datadog text routinely carries Swedish `åäö`, `é`, emoji, CJK and combining marks.
//! Texts are placed so those characters straddle the chunk boundary (1800 chars,
//! ±3) and the 1500-byte prompt excerpt limit (±5 bytes), in titles, messages and
//! service names. The pipeline must not panic, every stored payload must equal what the
//! adapters produced (no lossy re-encoding), chunks must reassemble to the stored text,
//! which holds the original message, and prompt excerpts must be valid prefixes of the
//! stored text.
//!
//! Logs are stored as pattern day documents whose text starts with a summary before the
//! sample message; every log here renders a summary of the same length, so the message
//! is placed relative to its measured offset ([`message_offset`]).

use super::support::{self, Corpus, FakeOpenAi, Store, at, openai::DIM};
use crate::{CHUNK_OVERLAP, CHUNK_SIZE};
use rag_core::log_patterns::{LogEvent, group};
use rag_core::rag_service::EXCERPT_MAX_BYTES;
use rag_core::text::TRUNCATION_MARKER;
use serde_json::{Value, json};

const NOW: &str = "2026-03-12T09:00:00Z";
const LOGGED_AT: &str = "2026-03-11T10:00:00.000Z";

/// Multibyte units: 2-, 3- and 4-byte chars, a combining sequence and a ZWJ emoji.
const UNITS: [&str; 5] = ["åäö", "決済", "🚀", "e\u{0301}", "👩\u{200D}💻"];

fn log(id: &str, service: &str, message: &str) -> Value {
    json!({"id": id, "type": "log", "attributes": {
        "service": service, "status": "error", "timestamp": LOGGED_AT,
        "message": message, "tags": ["env:prod"]
    }})
}

/// A fixed-width, letters-only prefix that gives every log its own pattern (padding
/// alone would not: patterns keep only the first 160 chars).
fn code(u: usize, i: usize) -> String {
    format!(
        "fall{}{} ",
        (b'g' + u as u8) as char,
        (b'g' + i as u8) as char
    )
}

/// Char and byte offset of the message in a single-log pattern document's text.
fn message_offset() -> (usize, usize) {
    let message = format!("{}{}", code(0, 0), "a".repeat(2000));
    let event = LogEvent {
        id: "x".into(),
        timestamp: at(LOGGED_AT),
        service: "chunk-svc".into(),
        environment: "prod".into(),
        status: "error".into(),
        message: message.clone(),
    };
    let text = group(&[event])[0]
        .to_document("https://app.datadoghq.eu")
        .text;
    let byte = text.find(&message).unwrap();
    (text[..byte].chars().count(), byte)
}

/// Message whose first multibyte unit starts at char `CHUNK_SIZE + d` of its document.
fn chunk_message(u: usize, d: usize) -> String {
    let pad = CHUNK_SIZE - 3 + d - message_offset().0 - code(u, d).len();
    format!(
        "{}{}{}{} slut",
        code(u, d),
        "a".repeat(pad),
        UNITS[u].repeat(3),
        "ö".repeat(50)
    )
}

/// Message whose first multibyte unit starts at byte `EXCERPT_MAX_BYTES - 5 + d`.
fn excerpt_message(u: usize, d: usize) -> String {
    let pad = EXCERPT_MAX_BYTES - 5 + d - message_offset().1 - code(u, d).len();
    format!("{}{}{}", code(u, d), "y".repeat(pad), UNITS[u].repeat(3))
}

/// The text the adapter renders for the single log `raw_id` of `corpus`.
fn document_text(corpus: &Corpus, raw_id: &str) -> String {
    let log = corpus.logs.iter().find(|l| l["id"] == raw_id).unwrap();
    let event = rag_core::datadog::log_event(log).unwrap();
    group(&[event])[0].to_document("").text
}

fn corpus() -> Corpus {
    let mut logs = vec![];
    for u in 0..UNITS.len() {
        // Around the chunk boundary, counted in chars like the chunker.
        for d in 0..=6 {
            logs.push(log(
                &format!("chunk-{u}-{d}"),
                "chunk-svc",
                &chunk_message(u, d),
            ));
        }
        // Around the prompt excerpt limit, counted in bytes like the excerpt.
        for d in 0..=6 {
            logs.push(log(
                &format!("excerpt-{u}-{d}"),
                &format!("excerpt-{u}"),
                &excerpt_message(u, d),
            ));
        }
    }
    // Multibyte service names and titles, including uppercase that is normalized.
    logs.push(log("svc-1", "Tjänst-Å", "Återförsök misslyckades"));
    Corpus::from_json(&json!({
        "monitors": [{"id": 1, "type": "query alert", "name": "Betalningar – översikt 📊 決済",
                      "query": "avg(last_5m):avg:payments.latency{*} > 1",
                      "message": "Kontrollera e\u{0301}tat 👩\u{200D}💻", "tags": ["service:betalning-åäö", "env:prod"]}],
        "incidents": [{"id": "inc-jp", "type": "incidents", "attributes": {
            "public_id": 7, "title": "決済ゲートウェイ タイムアウト 🚨", "created": "2026-03-11T10:00:00+00:00",
            "customer_impact_scope": "Kunder i Malmö och Tōkyō", "fields": {}}}],
        "logs": logs,
        // Descriptions and messages longer than the adapters' byte bounds, cut inside
        // multibyte runs.
        "serviceDefinitions": [{"id": "sd-1", "type": "service-definition", "attributes": {
            "meta": {}, "schema": {"schema-version": "v2.2", "dd-service": "Kassa-Å",
                "team": "Butik 🛒", "description": UNITS.concat().repeat(200),
                "links": [{"name": "Körbok 📖", "type": "runbook", "url": "https://wiki/kassa-å"}]}}}],
        "events": [{"id": "ev-1", "type": "event", "attributes": {
            "timestamp": LOGGED_AT, "tags": ["service:Kassa-Å", "env:prod"],
            "message": UNITS.concat().repeat(400),
            "attributes": {"title": "Driftsättning 決済 🚀 e\u{0301}",
                           "author": {"name": "Åsa 👩\u{200D}💻"}}}}]
    }))
}

async fn run_unicode(store: Store) {
    let now = at(NOW);
    let fake = FakeOpenAi::default();
    let openai = fake.start().await;
    let oa = support::openai_client(&openai.uri());
    let corpus = corpus();
    support::index(&corpus, oa.clone(), &store, now).await;

    // Payload equality: exactly the adapters' chunks come back from the store.
    let expected = support::expected_chunks(&corpus, now).await;
    let stored = support::stored_chunks(&store).await;
    assert_eq!(
        stored.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>()
    );
    for (id, s) in &stored {
        assert_eq!(
            serde_json::to_value(&s.doc).unwrap(),
            serde_json::to_value(&expected[id]).unwrap(),
            "{id} changed in the round trip"
        );
    }
    let svc = corpus.resolve("log_svc-1");
    assert_eq!(stored[&format!("{svc}#c0")].doc.service, "tjänst-å");
    assert_eq!(stored["monitor_1#c0"].doc.service, "betalning-åäö");
    assert_eq!(
        stored["incident_inc-jp#c0"].doc.title,
        "決済ゲートウェイ タイムアウト 🚨"
    );
    let catalog = &stored["catalog_kassa-å#c0"].doc;
    assert_eq!(catalog.service, "kassa-å");
    assert!(
        catalog
            .text
            .contains("Körbok 📖 (runbook): https://wiki/kassa-å")
    );
    assert!(
        catalog.text.contains(TRUNCATION_MARKER),
        "the description is bounded"
    );
    let change = &stored["change_ev-1#c0"].doc;
    assert_eq!(change.title, "Driftsättning 決済 🚀 e\u{0301}");
    assert_eq!(change.service, "kassa-å");
    assert_eq!(change.metadata["author"], "Åsa 👩\u{200D}💻");
    let change_text: String = (0..)
        .map_while(|i| stored.get(&format!("change_ev-1#c{i}")))
        .enumerate()
        .flat_map(|(i, c)| {
            c.doc
                .text
                .chars()
                .skip(if i == 0 { 0 } else { CHUNK_OVERLAP })
        })
        .collect();
    let body = change_text.split_once("\n\n").unwrap().1;
    assert!(body.ends_with(TRUNCATION_MARKER), "the message is bounded");
    assert!(body.len() <= rag_core::change_events::MESSAGE_MAX_BYTES + TRUNCATION_MARKER.len());
    assert!(
        UNITS
            .concat()
            .repeat(400)
            .starts_with(body.strip_suffix(TRUNCATION_MARKER).unwrap())
    );

    // Chunks respect the size limit and reassemble to the document text, whose sample
    // is the original message with its multibyte unit on the chunk boundary.
    for (u, unit) in UNITS.iter().enumerate() {
        for d in 0..=6 {
            let original = chunk_message(u, d);
            let parent = corpus.resolve(&format!("log_chunk-{u}-{d}"));
            let mut rebuilt = String::new();
            for i in 0.. {
                let Some(c) = stored.get(&format!("{parent}#c{i}")) else {
                    break;
                };
                assert!(c.doc.text.chars().count() <= CHUNK_SIZE);
                let skip = if i == 0 { 0 } else { CHUNK_OVERLAP };
                rebuilt.extend(c.doc.text.chars().skip(skip));
            }
            assert_eq!(
                rebuilt,
                document_text(&corpus, &format!("chunk-{u}-{d}")),
                "{parent}"
            );
            let at = rebuilt
                .find(&original)
                .unwrap_or_else(|| panic!("{parent}: no message"));
            let unit_at = rebuilt[..at].chars().count() + original.find(unit).unwrap();
            assert_eq!(unit_at, CHUNK_SIZE - 3 + d, "{parent}");
        }
    }

    // Excerpt-boundary documents put their multibyte unit around byte 1500.
    for (u, unit) in UNITS.iter().enumerate() {
        for d in 0..=6 {
            let text = document_text(&corpus, &format!("excerpt-{u}-{d}"));
            assert_eq!(text.find(unit), Some(EXCERPT_MAX_BYTES - 5 + d));
        }
    }

    // Every excerpt-boundary document goes through the prompt's 1500-byte cut.
    let base = support::spawn_api(oa, &store, now, None).await;
    for u in 0..UNITS.len() {
        let question = format!("excerpt {u} yyyy");
        let resp = support::ask(
            &base,
            &json!({"question": question, "service": format!("excerpt-{u}"), "kinds": ["logs"]}),
        )
        .await;
        assert_eq!(resp["sources"].as_array().unwrap().len(), 7, "{resp}");
        let prompt = fake.script.lock().unwrap().prompts[&question].clone();
        // An excerpt ends before the next document or, for the last one, the
        // instructions (pattern texts contain blank lines themselves).
        let excerpts: Vec<&str> = prompt
            .split("Excerpt:\n")
            .skip(1)
            .map(|rest| {
                let end = rest
                    .find("\n\n[DOC #")
                    .or_else(|| rest.find("\n\n\n"))
                    .unwrap();
                &rest[..end]
            })
            .collect();
        assert_eq!(excerpts.len(), 7);
        for (source, excerpt) in resp["sources"].as_array().unwrap().iter().zip(&excerpts) {
            let full = &stored[&format!("{}#c0", source["id"].as_str().unwrap())]
                .doc
                .text;
            let head = excerpt.strip_suffix(TRUNCATION_MARKER).unwrap_or(excerpt);
            assert!(
                full.starts_with(head),
                "excerpt of {source} is not a prefix"
            );
            assert!(head.len() <= EXCERPT_MAX_BYTES);
            assert_eq!(
                head.len() < full.len(),
                excerpt.ends_with(TRUNCATION_MARKER)
            );
        }
    }

    // Multibyte titles reach `sources` unchanged.
    let resp = support::ask(
        &base,
        &json!({"question": "決済 タイムアウト", "kinds": ["incident", "monitor"]}),
    )
    .await;
    let titles: Vec<&str> = resp["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["title"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"決済ゲートウェイ タイムアウト 🚨"),
        "{titles:?}"
    );
    assert!(
        titles.contains(&"Betalningar – översikt 📊 決済"),
        "{titles:?}"
    );

    // The new kinds with multibyte service names reach `sources` and the prompt.
    let resp = support::ask(
        &base,
        &json!({"question": "Kassa-Å körbok driftsättning", "service": "Kassa-Å",
                "kinds": ["catalog", "change"]}),
    )
    .await;
    let mut ids = support::source_ids(&resp);
    ids.sort();
    assert_eq!(ids, ["catalog_kassa-å", "change_ev-1"], "{resp}");
    store.finish().await;
}

#[tokio::test]
async fn unicode_round_trip_in_memory() {
    run_unicode(Store::fake(DIM).await).await;
}

#[tokio::test]
#[ignore = "requires QDRANT_TEST_ENDPOINT pointing at an isolated Qdrant server"]
async fn unicode_round_trip_real_qdrant() {
    run_unicode(Store::real(DIM).await).await;
}
