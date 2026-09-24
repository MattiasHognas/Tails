//! Multibyte text through the whole pipeline.
//!
//! Datadog text routinely carries Swedish `åäö`, `é`, emoji, CJK and combining marks.
//! Texts are placed so those characters straddle the chunk boundary (1800 chars,
//! ±3) and the 1500-byte prompt excerpt limit (±5 bytes), in titles, messages and
//! service names. The pipeline must not panic, every stored payload must equal what the
//! adapters produced (no lossy re-encoding), chunks must reassemble to the original
//! text, and prompt excerpts must be valid prefixes of the stored text.

use super::support::{self, Corpus, FakeOpenAi, Store, at, openai::DIM};
use crate::{CHUNK_OVERLAP, CHUNK_SIZE};
use rag_core::rag_service::EXCERPT_MAX_BYTES;
use rag_core::text::TRUNCATION_MARKER;
use serde_json::{Value, json};

const NOW: &str = "2026-03-12T09:00:00Z";

/// Multibyte units: 2-, 3- and 4-byte chars, a combining sequence and a ZWJ emoji.
const UNITS: [&str; 5] = ["åäö", "決済", "🚀", "e\u{0301}", "👩\u{200D}💻"];

fn log(id: &str, service: &str, message: &str) -> Value {
    json!({"id": id, "type": "log", "attributes": {
        "service": service, "status": "error", "timestamp": "2026-03-11T10:00:00.000Z",
        "message": message, "tags": ["env:prod"]
    }})
}

fn corpus() -> Corpus {
    let mut logs = vec![];
    for (u, unit) in UNITS.iter().enumerate() {
        // Around the chunk boundary, counted in chars like the chunker.
        for k in CHUNK_SIZE - 3..=CHUNK_SIZE + 3 {
            let text = format!("{}{}{} slut", "a".repeat(k), unit.repeat(3), "ö".repeat(50));
            logs.push(log(&format!("chunk-{u}-{k}"), "chunk-svc", &text));
        }
        // Around the prompt excerpt limit, counted in bytes like the excerpt.
        for b in EXCERPT_MAX_BYTES - 5..=EXCERPT_MAX_BYTES + 1 {
            let text = format!("{}{}", "y".repeat(b), unit.repeat(3));
            logs.push(log(
                &format!("excerpt-{u}-{b}"),
                &format!("excerpt-{u}"),
                &text,
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
        "logs": logs
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
    assert_eq!(stored["log_svc-1#c0"].doc.service, "tjänst-å");
    assert_eq!(stored["monitor_1#c0"].doc.service, "betalning-åäö");
    assert_eq!(
        stored["incident_inc-jp#c0"].doc.title,
        "決済ゲートウェイ タイムアウト 🚨"
    );

    // Chunks respect the size limit and reassemble to the original message.
    for (u, unit) in UNITS.iter().enumerate() {
        for k in CHUNK_SIZE - 3..=CHUNK_SIZE + 3 {
            let original = format!("{}{}{} slut", "a".repeat(k), unit.repeat(3), "ö".repeat(50));
            let parent = format!("log_chunk-{u}-{k}");
            let mut rebuilt = String::new();
            for i in 0.. {
                let Some(c) = stored.get(&format!("{parent}#c{i}")) else {
                    break;
                };
                assert!(c.doc.text.chars().count() <= CHUNK_SIZE);
                let skip = if i == 0 { 0 } else { CHUNK_OVERLAP };
                rebuilt.extend(c.doc.text.chars().skip(skip));
            }
            assert_eq!(rebuilt, original, "{parent} does not reassemble");
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
        let excerpts: Vec<&str> = prompt
            .split("Excerpt:\n")
            .skip(1)
            .map(|rest| rest.split("\n\n").next().unwrap())
            .collect();
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
