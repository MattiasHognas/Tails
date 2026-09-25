use anyhow::{Context, Result};
use rag_core::{
    chunk::{chunk, chunk_id, content_hash},
    domain::{Hit, RagDocument, SourceKind},
    qdrant::{
        Fusion, QPoint, Qdrant, Rrf, RrfWeights, SYNC_ID_KEY, SearchQuery, StoredPointState,
        point_id, surplus_chunks_filter, unsynced_filter,
    },
    retrieval::RetrievalScope,
    sparse::query_vector,
};
use serde_json::json;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Run against an isolated, real Qdrant server, never a production collection.
#[tokio::test]
#[ignore = "requires QDRANT_TEST_ENDPOINT pointing at a real Qdrant server"]
async fn qdrant_roundtrip() -> Result<()> {
    let endpoint = std::env::var("QDRANT_TEST_ENDPOINT")
        .context("set QDRANT_TEST_ENDPOINT to an isolated Qdrant server")?;
    let collection = format!(
        "tails_contract_{}_{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    );
    let mut qdrant = Qdrant::new(endpoint, collection);
    qdrant.http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let collection_url = format!("{}/collections/{}", qdrant.endpoint, qdrant.collection);
    let mut old = qdrant.clone();
    old.collection = format!("{}_old", qdrant.collection);
    let old_url = format!("{}/collections/{}", old.endpoint, old.collection);

    // Clean up even when a request or assertion fails.
    let test_qdrant = qdrant.clone();
    let outcome = tokio::spawn(async move {
        check_collection_setup(&test_qdrant, &old).await?;
        check_roundtrip(&test_qdrant).await?;
        check_scope_filter(&test_qdrant).await?;
        check_hybrid_search(&test_qdrant).await?;
        check_incremental_indexing_calls(&test_qdrant).await
    })
    .await;
    let cleanup = qdrant.http.delete(&collection_url).send().await;
    let _ = qdrant.http.delete(&old_url).send().await;
    outcome.context("Qdrant round-trip assertion failed")??;
    cleanup?.error_for_status()?;
    Ok(())
}

/// The indexer's collection setup: a missing collection is reported and created with
/// named dense + sparse vectors, which then pass the check; a collection with the old
/// single unnamed vector is refused.
async fn check_collection_setup(qdrant: &Qdrant, old: &Qdrant) -> Result<()> {
    assert!(!qdrant.check_collection().await?);
    qdrant.create_collection(3).await?;
    assert!(qdrant.check_collection().await?);

    let old_url = format!("{}/collections/{}", old.endpoint, old.collection);
    old.http
        .put(&old_url)
        .json(&json!({"vectors": {"size": 3, "distance": "Cosine"}}))
        .send()
        .await?
        .error_for_status()?;
    let err = old.check_collection().await.unwrap_err();
    assert!(err.to_string().contains("new"), "{err}");
    Ok(())
}

/// A hybrid search with one query: its dense vector and the keyword vector of `text`.
async fn search(
    qdrant: &Qdrant,
    dense: &[f32],
    text: &str,
    filter: Option<serde_json::Value>,
) -> Result<Vec<Hit>> {
    let query = SearchQuery {
        dense: dense.to_vec(),
        sparse: query_vector(text),
    };
    Ok(qdrant.hybrid_search(&[query], 100, filter).await?)
}

/// Keyword search finds an exact identifier the dense vector misses, both questions
/// are fused in one query, and scores are normalized RRF (or DBSF).
async fn check_hybrid_search(qdrant: &Qdrant) -> Result<()> {
    let doc = |id: &str, text: &str| RagDocument {
        id: id.into(),
        title: id.into(),
        text: text.into(),
        source_uri: format!("https://example.com/{id}"),
        kind: SourceKind::Logs,
        timestamp: None,
        service: "hybrid-svc".into(),
        environment: "prod".into(),
        metadata: serde_json::Map::new(),
    };
    let exact = doc("log_exact#c0", "upstream closed: ERR_CONN_RESET");
    let similar = doc("log_similar#c0", "connection refused by upstream");
    let other = doc("log_other#c0", "disk quota exceeded");
    qdrant
        .upsert(vec![
            QPoint::from_document(&exact, vec![1.0, 0.0, 0.0]),
            QPoint::from_document(&similar, vec![0.0, 1.0, 0.0]),
            QPoint::from_document(&other, vec![0.0, 0.0, 1.0]),
        ])
        .await?;
    let filter = json!({"must": [{"key": "Service", "match": {"value": "hybrid-svc"}}]});
    let ranked = |hits: Vec<Hit>| -> Vec<(String, f32)> {
        hits.into_iter().map(|h| (h.doc.id, h.score)).collect()
    };

    // Dense ranks similar, exact, other; only exact shares a keyword. Two lists, k = 2:
    // exact 1/3 + 1/2 and similar 1/2, of the best possible 2 × 1/2.
    let hits = search(
        qdrant,
        &[0.2, 1.0, 0.0],
        "why ERR_CONN_RESET?",
        Some(filter.clone()),
    )
    .await?;
    let got = ranked(hits);
    assert_eq!(got[0].0, "log_exact#c0", "{got:?}");
    assert!((got[0].1 - 5.0 / 6.0).abs() < 1e-5, "{got:?}");
    assert_eq!(got[1].0, "log_similar#c0", "{got:?}");
    assert!((got[1].1 - 0.5).abs() < 1e-5, "{got:?}");
    assert_eq!(got.len(), 3);

    // The same search fused by DBSF. Dense cosines similar 0.981, exact 0.196, other 0
    // normalize by mean ± 3 sample σ to 0.689, 0.437, 0.374; the keyword list holds
    // exact alone, 0.5. Of the best possible 2 (3σ up in both lists): exact 0.4685,
    // similar 0.3445, other 0.1870.
    let mut dbsf = qdrant.clone();
    dbsf.hybrid.fusion = Fusion::Dbsf;
    let got = ranked(
        search(
            &dbsf,
            &[0.2, 1.0, 0.0],
            "why ERR_CONN_RESET?",
            Some(filter.clone()),
        )
        .await?,
    );
    let want = [
        ("log_exact#c0", 0.4685),
        ("log_similar#c0", 0.34449),
        ("log_other#c0", 0.18701),
    ];
    assert_eq!(got.len(), want.len(), "{got:?}");
    for ((id, score), (want_id, want_score)) in got.iter().zip(want) {
        assert_eq!(id, want_id, "{got:?}");
        assert!((score - want_score).abs() < 1e-4, "{got:?}");
    }

    // RRF with another k and with weights, normalized by the best possible score. Dense
    // ranks similar, exact, other; keyword has exact alone. A point at 0-based rank r
    // of a list of weight w scores 1 / ((r + 1) / w + k - 1).
    let cases = [
        // k = 10: exact 1/11 + 1/10, similar 1/10, other 1/12, of the best 2/10.
        (10, None, [0.954_545, 0.5, 0.416_667]),
        // Keyword twice the dense weight, k = 2: exact 1/3 + 1/1.5, similar 1/2,
        // other 1/4, of the best 1/2 + 1/1.5.
        (2, Some((1.0, 2.0)), [0.857_143, 0.428_571, 0.214_286]),
        // Dense twice the keyword weight: exact 1/2 + 1/2, similar 1/1.5, other 1/2.5.
        (2, Some((2.0, 1.0)), [0.857_143, 0.571_429, 0.342_857]),
    ];
    for (k, weights, want) in cases {
        let mut rrf = qdrant.clone();
        rrf.hybrid.fusion = Fusion::Rrf(Rrf {
            k,
            weights: weights.map(|(dense, keyword)| RrfWeights { dense, keyword }),
        });
        let got = ranked(
            search(
                &rrf,
                &[0.2, 1.0, 0.0],
                "why ERR_CONN_RESET?",
                Some(filter.clone()),
            )
            .await?,
        );
        let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            ids,
            ["log_exact#c0", "log_similar#c0", "log_other#c0"],
            "{got:?}"
        );
        for ((_, score), want) in got.iter().zip(want) {
            assert!((score - want).abs() < 1e-5, "k {k} {weights:?}: {got:?}");
        }
    }

    // The question and a rewrite in one query: four lists. `exact` is first by the
    // question's dense and keyword searches and third by the rewrite's dense one;
    // `other` the other way round (they tie, in any order); `similar` is second twice.
    let queries = [
        SearchQuery {
            dense: vec![1.0, 0.2, 0.0],
            sparse: query_vector("ERR_CONN_RESET"),
        },
        SearchQuery {
            dense: vec![0.0, 0.1, 1.0],
            sparse: query_vector("disk quota"),
        },
    ];
    let got = ranked(qdrant.hybrid_search(&queries, 100, Some(filter)).await?);
    let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{got:?}");
    assert!(ids[..2].contains(&"log_exact#c0") && ids[..2].contains(&"log_other#c0"));
    // exact: 1/2 + 1/2 + 1/4 = 1.25 and similar 1/3 + 1/3, of the best possible 4 × 1/2.
    let exact_score = got.iter().find(|(id, _)| id == "log_exact#c0").unwrap().1;
    assert!((exact_score - 1.25 / 2.0).abs() < 1e-5, "{got:?}");
    assert_eq!(ids[2], "log_similar#c0");
    assert!((got[2].1 - (2.0 / 3.0) / 2.0).abs() < 1e-5, "{got:?}");
    Ok(())
}

async fn check_roundtrip(qdrant: &Qdrant) -> Result<()> {
    let source = RagDocument {
        id: "monitor_123".into(),
        title: "Återkommande fel".into(),
        text: "Timeout i betalningstjänsten. Kontrollera anslutningen.".into(),
        source_uri: "https://app.datadoghq.eu/monitors/123".into(),
        kind: SourceKind::Monitor,
        timestamp: Some("2026-09-24T10:00:00Z".into()),
        service: "payments".into(),
        environment: "prod".into(),
        metadata: json!({"tags": ["team:payments"], "severity": "SEV-2"})
            .as_object()
            .unwrap()
            .clone(),
    };
    let mut chunks = chunk(32, 8, &source);
    assert!(chunks.len() > 1);
    // Exercise an absent timestamp as well as a populated one.
    chunks[1].timestamp = None;
    let vector = vec![1.0, 0.0, 0.0];
    let points: Vec<_> = chunks
        .iter()
        .map(|doc| QPoint::from_document(doc, vector.clone()))
        .collect();
    assert_ne!(points[0].id, points[1].id);
    qdrant.upsert(points.clone()).await?;
    qdrant.upsert(points).await?;

    // An equally relevant candidate in another environment must be filtered out.
    let mut other = chunks[0].clone();
    other.id = "monitor_staging#c0".into();
    other.environment = "staging".into();
    qdrant
        .upsert(vec![QPoint::from_document(&other, vector.clone())])
        .await?;

    let filter = json!({"must": [
        {"key": "Service", "match": {"value": "payments"}},
        {"key": "Environment", "match": {"value": "prod"}}
    ]});
    let hits = search(qdrant, &vector, "timeout", Some(filter.clone())).await?;
    assert_eq!(
        hits.len(),
        chunks.len(),
        "repeat upserts must not duplicate chunks"
    );
    for expected in &chunks {
        let actual = &hits.iter().find(|h| h.doc.id == expected.id).unwrap().doc;
        assert_eq!(
            serde_json::to_value(actual)?,
            serde_json::to_value(expected)?
        );
        assert_eq!(actual.metadata["chunk_of"], source.id);
    }

    // A content update must replace the same point, preserving logical identity.
    let original_id = QPoint::from_document(&chunks[0], vector.clone()).id;
    chunks[0].text = "Updated evidence".into();
    let updated = QPoint::from_document(&chunks[0], vector.clone());
    assert_eq!(updated.id, original_id);
    qdrant.upsert(vec![updated]).await?;
    let hits = search(qdrant, &vector, "timeout", Some(filter)).await?;
    assert_eq!(hits.len(), chunks.len());
    let actual = &hits.iter().find(|h| h.doc.id == chunks[0].id).unwrap().doc;
    assert_eq!(
        serde_json::to_value(actual)?,
        serde_json::to_value(&chunks[0])?
    );
    Ok(())
}

/// The retrieval scope filter must select events inside the window while keeping
/// timestamp-less documents and configuration kinds (monitor/dashboard/SLO), and log
/// pattern days whose first..last log overlaps the window.
async fn check_scope_filter(qdrant: &Qdrant) -> Result<()> {
    let doc = |id: &str, kind: SourceKind, ts: Option<&str>| RagDocument {
        id: id.into(),
        title: id.into(),
        text: id.into(),
        source_uri: format!("https://example.com/{id}"),
        kind,
        timestamp: ts.map(str::to_string),
        service: "scope-svc".into(),
        environment: "prod".into(),
        metadata: serde_json::Map::new(),
    };
    // A log pattern day spans [Metadata.first_seen, Timestamp] within one UTC day.
    let pattern = |id: &str, first: &str, last: &str| {
        let mut d = doc(id, SourceKind::Logs, Some(last));
        d.metadata.insert("first_seen".into(), json!(first));
        d
    };
    let docs = [
        // Last log after the window: only `first_seen` places it inside.
        pattern(
            "pattern_ending_after",
            "2026-09-23T21:00:00.000Z",
            "2026-09-23T23:00:00.000Z",
        ),
        pattern(
            "pattern_after",
            "2026-09-23T22:00:00.000Z",
            "2026-09-23T23:30:00.000Z",
        ),
        pattern(
            "pattern_before",
            "2026-09-21T10:00:00.000Z",
            "2026-09-22T21:59:59.999Z",
        ),
        doc(
            "incident_inside",
            SourceKind::Incident,
            Some("2026-09-23T10:00:00.123Z"),
        ),
        doc(
            "incident_before",
            SourceKind::Incident,
            Some("2026-09-22T21:59:59Z"),
        ),
        doc(
            "incident_at_end",
            SourceKind::Incident,
            Some("2026-09-23T22:00:00Z"),
        ),
        doc("incident_untimed", SourceKind::Incident, None),
        doc("monitor", SourceKind::Monitor, None),
        doc(
            "dashboard_old",
            SourceKind::Dashboard,
            Some("2020-01-01T00:00:00Z"),
        ),
        doc("slo", SourceKind::SLO, None),
        // Metric catalog entries carry the indexing time, not an event time.
        doc(
            "metric_old",
            SourceKind::Metrics,
            Some("2020-01-01T00:00:00Z"),
        ),
    ];
    let vector = vec![0.0, 1.0, 0.0];
    let points = docs
        .iter()
        .map(|d| QPoint::from_document(d, vector.clone()))
        .collect();
    qdrant.upsert(points).await?;

    let mut scope = RetrievalScope {
        service: Some("scope-svc".into()),
        environment: Some("prod".into()),
        from_utc: Some("2026-09-22T22:00:00Z".parse()?),
        to_utc: Some("2026-09-23T22:00:00Z".parse()?),
        kinds: vec![],
    };
    let ids = |hits: Vec<Hit>| {
        let mut ids: Vec<String> = hits.into_iter().map(|h| h.doc.id).collect();
        ids.sort();
        ids
    };
    let hits = search(qdrant, &vector, "log", scope.to_qdrant_filter()).await?;
    assert_eq!(
        ids(hits),
        [
            "dashboard_old",
            "incident_inside",
            "incident_untimed",
            "metric_old",
            "monitor",
            "pattern_ending_after",
            "slo"
        ]
    );

    scope.kinds = vec![SourceKind::Logs, SourceKind::SLO];
    let hits = search(qdrant, &vector, "log", scope.to_qdrant_filter()).await?;
    assert_eq!(ids(hits), ["pattern_ending_after", "slo"]);
    Ok(())
}

/// The calls incremental indexing relies on: retrieve bookkeeping by point ID, mark points
/// with set_payload, and count/delete by the surplus-chunk and unsynced filters.
async fn check_incremental_indexing_calls(qdrant: &Qdrant) -> Result<()> {
    let doc = |id: &str, kind: SourceKind, text: &str| RagDocument {
        id: id.into(),
        title: id.into(),
        text: text.into(),
        source_uri: format!("https://example.com/{id}"),
        kind,
        timestamp: None,
        service: "incremental-svc".into(),
        environment: "prod".into(),
        metadata: serde_json::Map::new(),
    };
    let vector = vec![0.0, 0.0, 1.0];
    let store = |docs: &[RagDocument], sync_id: Option<&str>| {
        let points: Vec<QPoint> = docs
            .iter()
            .flat_map(|d| {
                let hash = content_hash(d, 10, 0, "contract-model");
                let chunks = chunk(10, 0, d);
                let count = chunks.len() as u64;
                chunks.into_iter().map(move |c| (c, hash.clone(), count))
            })
            .map(|(c, hash, count)| {
                let mut p = QPoint::from_document(&c, vector.clone());
                p.payload.content_hash = Some(hash);
                p.payload.chunk_count = Some(count);
                p.payload.sync_id = sync_id.map(str::to_string);
                p
            })
            .collect();
        qdrant.upsert(points)
    };

    // Two 4-chunk monitors, a 1-chunk monitor, a 4-chunk log and a legacy monitor point
    // without bookkeeping.
    let long = doc("monitor_inc", SourceKind::Monitor, &"m".repeat(40));
    let other = doc("monitor_other", SourceKind::Monitor, &"o".repeat(40));
    let gone = doc("monitor_gone", SourceKind::Monitor, "gone");
    let log = doc("log_inc", SourceKind::Logs, &"l".repeat(40));
    store(&[long.clone(), other, gone], Some("run-1")).await?;
    store(&[log], None).await?;
    let legacy_doc = doc("monitor_legacy", SourceKind::Monitor, "legacy");
    let legacy = QPoint::from_document(&chunk(10, 0, &legacy_doc)[0], vector.clone());
    qdrant.upsert(vec![legacy.clone()]).await?;

    // Retrieve: existing points return their bookkeeping, missing IDs are left out.
    let c0 = point_id(&chunk_id("monitor_inc", 0));
    let missing = point_id(&chunk_id("monitor_inc", 99));
    let mut states = qdrant.retrieve_states(&[c0, legacy.id, missing]).await?;
    states.sort_by_key(|s| s.id != c0);
    assert_eq!(
        states,
        [
            StoredPointState {
                id: c0,
                content_hash: Some(content_hash(&long, 10, 0, "contract-model")),
                chunk_count: Some(4),
            },
            StoredPointState {
                id: legacy.id,
                content_hash: None,
                chunk_count: None,
            },
        ]
    );

    // Metadata of existing points comes back whole; missing IDs are left out.
    let metadata = qdrant.retrieve_metadata(&[c0, missing]).await?;
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata[0].0, c0);
    assert_eq!(metadata[0].1, chunk(10, 0, &long)[0].metadata);

    // Shrink: deleting chunks 2.. of one monitor leaves its chunks 0 and 1 and every
    // other document's chunks.
    let surplus = surplus_chunks_filter(&SourceKind::Monitor, "monitor_inc", 2);
    assert_eq!(qdrant.count(surplus.clone()).await?, 2);
    qdrant.delete_by_filter(surplus).await?;
    let ids_of =
        |doc_id: &str| -> Vec<_> { (0..4).map(|i| point_id(&chunk_id(doc_id, i))).collect() };
    let monitor_ids = ids_of("monitor_inc");
    let mut left: Vec<_> = qdrant
        .retrieve_states(&monitor_ids)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    left.sort();
    let mut expected = monitor_ids[..2].to_vec();
    expected.sort();
    assert_eq!(left, expected);
    assert_eq!(
        qdrant
            .retrieve_states(&ids_of("monitor_other"))
            .await?
            .len(),
        4
    );
    assert_eq!(qdrant.retrieve_states(&ids_of("log_inc")).await?.len(), 4);

    // Full sync: mark the monitors seen in run-2, then drop every unmarked monitor.
    let mut seen = monitor_ids[..2].to_vec();
    seen.extend(ids_of("monitor_other"));
    qdrant
        .set_payload(&seen, json!({ SYNC_ID_KEY: "run-2" }))
        .await?;
    let stale = qdrant
        .count(unsynced_filter(&SourceKind::Monitor, "run-2"))
        .await?;
    // Monitors from the earlier checks share the collection and are unmarked too.
    let earlier_monitors = qdrant
        .count(json!({
            "must": [{"key": "Kind", "match": {"value": "monitor"}}],
            "must_not": [{"key": "Service", "match": {"value": "incremental-svc"}}]
        }))
        .await?;
    assert_eq!(
        stale,
        earlier_monitors + 2,
        "monitor_gone (run-1) and the legacy point"
    );
    qdrant
        .delete_by_filter(unsynced_filter(&SourceKind::Monitor, "run-2"))
        .await?;
    assert_eq!(
        qdrant
            .count(unsynced_filter(&SourceKind::Monitor, "run-2"))
            .await?,
        0
    );
    let kept = search(
        qdrant,
        &vector,
        "monitor",
        Some(json!({"must": [{"key": "Service", "match": {"value": "incremental-svc"}}]})),
    )
    .await?;
    let mut kept: Vec<_> = kept.into_iter().map(|h| h.doc.id).collect();
    kept.sort();
    assert_eq!(
        kept,
        [
            "log_inc#c0",
            "log_inc#c1",
            "log_inc#c2",
            "log_inc#c3",
            "monitor_inc#c0",
            "monitor_inc#c1",
            "monitor_other#c0",
            "monitor_other#c1",
            "monitor_other#c2",
            "monitor_other#c3"
        ],
        "windowed kinds are never swept by a full sync"
    );
    Ok(())
}
