use anyhow::{Context, Result};
use rag_core::{
    chunk::{chunk, chunk_id, content_hash},
    domain::{RagDocument, SourceKind},
    qdrant::{
        QPoint, Qdrant, SYNC_ID_KEY, StoredPointState, point_id, surplus_chunks_filter,
        unsynced_filter,
    },
    retrieval::RetrievalScope,
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
    qdrant
        .http
        .put(&collection_url)
        .json(&json!({"vectors": {"size": 3, "distance": "Cosine"}}))
        .send()
        .await?
        .error_for_status()?;

    // Clean up even when a request or assertion fails.
    let test_qdrant = qdrant.clone();
    let outcome = tokio::spawn(async move {
        check_roundtrip(&test_qdrant).await?;
        check_scope_filter(&test_qdrant).await?;
        check_incremental_indexing_calls(&test_qdrant).await
    })
    .await;
    let cleanup = qdrant.http.delete(&collection_url).send().await;
    outcome.context("Qdrant round-trip assertion failed")??;
    cleanup?.error_for_status()?;
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
    let hits = qdrant
        .search(vector.clone(), 100, Some(filter.clone()))
        .await?;
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
    let hits = qdrant.search(vector, 100, Some(filter)).await?;
    assert_eq!(hits.len(), chunks.len());
    let actual = &hits.iter().find(|h| h.doc.id == chunks[0].id).unwrap().doc;
    assert_eq!(
        serde_json::to_value(actual)?,
        serde_json::to_value(&chunks[0])?
    );
    Ok(())
}

/// The retrieval scope filter must select events inside the window while keeping
/// timestamp-less documents and configuration kinds (monitor/dashboard/SLO).
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
    let docs = [
        doc(
            "log_inside",
            SourceKind::Logs,
            Some("2026-09-23T10:00:00.123Z"),
        ),
        doc("log_before", SourceKind::Logs, Some("2026-09-22T21:59:59Z")),
        doc("log_at_end", SourceKind::Logs, Some("2026-09-23T22:00:00Z")),
        doc("log_untimed", SourceKind::Logs, None),
        doc("monitor", SourceKind::Monitor, None),
        doc(
            "dashboard_old",
            SourceKind::Dashboard,
            Some("2020-01-01T00:00:00Z"),
        ),
        doc("slo", SourceKind::SLO, None),
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
    let ids = |hits: Vec<rag_core::domain::Hit>| {
        let mut ids: Vec<String> = hits.into_iter().map(|h| h.doc.id).collect();
        ids.sort();
        ids
    };
    let hits = qdrant
        .search(vector.clone(), 100, scope.to_qdrant_filter())
        .await?;
    assert_eq!(
        ids(hits),
        [
            "dashboard_old",
            "log_inside",
            "log_untimed",
            "monitor",
            "slo"
        ]
    );

    scope.kinds = vec![SourceKind::Logs, SourceKind::SLO];
    let hits = qdrant.search(vector, 100, scope.to_qdrant_filter()).await?;
    assert_eq!(ids(hits), ["log_inside", "log_untimed", "slo"]);
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
    let kept = qdrant
        .search(
            vector,
            100,
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
