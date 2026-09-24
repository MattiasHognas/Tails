use anyhow::{Context, Result};
use rag_core::{
    chunk::chunk,
    domain::{RagDocument, SourceKind},
    qdrant::{QPoint, Qdrant},
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
        check_scope_filter(&test_qdrant).await
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
