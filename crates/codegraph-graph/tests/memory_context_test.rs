// ABOUTME: Optional memory graph context remains bounded, directed, and project-scoped.
// ABOUTME: Uses an isolated in-memory graph without providers or repository indexing.
#![cfg(feature = "surrealdb")]
use codegraph_graph::{GraphFunctions, SurrealDbConfig, SurrealDbStorage};
#[tokio::test]
async fn memory_context_retains_edge_provenance_without_crossing_project_scope() {
    let storage = SurrealDbStorage::new(SurrealDbConfig {
        connection: "mem://".into(),
        username: None,
        password: None,
        ..Default::default()
    })
    .await
    .unwrap();
    storage
        .db()
        .query("DEFINE TABLE file_metadata SCHEMALESS;")
        .await
        .unwrap()
        .check()
        .unwrap();
    storage.db().query("CREATE nodes:root CONTENT {project_id:'memory-test',name:'root',file_path:'src/lib.rs',start_line:1,end_line:2}; CREATE nodes:neighbor CONTENT {project_id:'memory-test',name:'neighbor',file_path:'src/lib.rs',start_line:3,end_line:4}; CREATE nodes:foreign CONTENT {project_id:'other',name:'foreign'}; CREATE edges:local CONTENT {project_id:'memory-test',from:nodes:root,to:nodes:neighbor,edge_type:'calls',metadata:{resolution:'exact'}}; CREATE edges:foreign CONTENT {project_id:'other',from:nodes:root,to:nodes:foreign}; CREATE project_metadata:ready CONTENT {project_id:'memory-test',metadata:{input_fingerprint:'ready-v1'}};").await.unwrap().check().unwrap();
    let graph = GraphFunctions::new_with_project_id(storage.db(), "memory-test");
    let context = graph
        .memory_code_context(&["nodes:root".into()])
        .await
        .unwrap();
    let nodes = context["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("Unexpected memory code context: {context}"));
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|v| v["name"] != "foreign"));
    let edges = context["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "nodes:root");
    assert_eq!(edges[0]["to"], "nodes:neighbor");
    assert_eq!(edges[0]["metadata"]["resolution"], "exact");
    assert_eq!(context["input_fingerprint"], "ready-v1");
}

#[tokio::test]
async fn native_memory_projection_joins_nodes_and_rebuilds_without_becoming_memory_authority() {
    use codegraph_core::memory_projection::*;
    let storage = SurrealDbStorage::new(SurrealDbConfig {
        connection: "mem://".into(),
        username: None,
        password: None,
        ..Default::default()
    })
    .await
    .unwrap();
    storage.db().query("CREATE nodes:root CONTENT {project_id:'p',name:'root'}; CREATE nodes:neighbor CONTENT {project_id:'p',name:'neighbor'}; CREATE nodes:foreign CONTENT {project_id:'other',name:'foreign'}; CREATE edges:call CONTENT {project_id:'p',from:nodes:root,to:nodes:neighbor,edge_type:'calls',metadata:{resolution:'exact'}}; CREATE edges:bad CONTENT {project_id:'other',from:nodes:root,to:nodes:foreign}; CREATE project_metadata:ready CONTENT {project_id:'p',metadata:{input_fingerprint:'v1'}};").await.unwrap().check().unwrap();
    let graph = GraphFunctions::new_with_project_id(storage.db(), "p");
    let cursor = graph.memory_projection_cursor("source").await.unwrap();
    assert_eq!(cursor, ProjectionCursor::default());
    let batch = ProjectionBatch {
        source_id: "source".into(),
        cursor: ProjectionCursor {
            generation: 1,
            memory_id: "z".into(),
        },
        complete: true,
        changes: vec![ProjectionChange {
            memory_id: "claim".into(),
            revision: 1,
            generation: 1,
            enabled: true,
            anchors: vec![
                ProjectionAnchor {
                    graph_project_id: "p".into(),
                    node_id: "nodes:neighbor".into(),
                    supporting: true,
                },
                ProjectionAnchor {
                    graph_project_id: "p".into(),
                    node_id: "nodes:foreign".into(),
                    supporting: false,
                },
            ],
        }],
    };
    graph
        .apply_memory_projection(&batch, &cursor)
        .await
        .unwrap();
    assert_eq!(
        graph.memory_projection_cursor("source").await.unwrap(),
        batch.cursor
    );
    assert!(
        graph
            .apply_memory_projection(&batch, &cursor)
            .await
            .is_err(),
        "A stale sync cannot overwrite a newer cursor"
    );
    let candidates = graph
        .memory_projection_candidates("source", &["nodes:root".into()])
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["memory_id"], "claim");
    assert_eq!(candidates[0]["code_hop_count"], 1);
    assert_eq!(candidates[0]["paths"][0]["direction"], "outgoing");
    assert_eq!(
        candidates[0]["paths"][0]["code_edge"]["metadata"]["resolution"],
        "exact"
    );
    let direct = graph
        .memory_projection_candidates("source", &["nodes:neighbor".into()])
        .await
        .unwrap();
    assert_eq!(direct[0]["code_hop_count"], 0);
    let mut fan_roots = Vec::new();
    for index in 0..6 {
        storage.db().query(format!("CREATE nodes:fan{index} CONTENT {{project_id:'p'}}; CREATE edges:fan{index} CONTENT {{project_id:'p',from:nodes:fan{index},to:nodes:neighbor,edge_type:'calls'}};")).await.unwrap().check().unwrap();
        fan_roots.push(format!("nodes:fan{index}"));
    }
    let bounded = graph
        .memory_projection_candidates("source", &fan_roots)
        .await
        .unwrap();
    assert_eq!(bounded[0]["paths"].as_array().unwrap().len(), 4);
    assert_eq!(bounded[0]["paths_truncated"], true);
    assert!(
        graph
            .memory_projection_candidates("source", &["nodes:foreign".into()])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        graph
            .memory_projection_candidates("another-source", &["nodes:root".into()])
            .await
            .unwrap()
            .is_empty()
    );
    let raw:Vec<serde_json::Value>=storage.db().query("SELECT memory_id,revision,->memory_code_anchor->nodes.name AS symbols FROM memory_projection").await.unwrap().check().unwrap().take(0).unwrap();
    assert_eq!(raw[0]["symbols"], serde_json::json!(["neighbor"]));
    storage
        .db()
        .query("UPDATE project_metadata:ready SET metadata.input_fingerprint='v2';")
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(
        graph.memory_projection_cursor("source").await.unwrap(),
        ProjectionCursor::default()
    );
    graph
        .apply_memory_projection(&batch, &ProjectionCursor::default())
        .await
        .unwrap();
    let deletion = ProjectionBatch {
        source_id: "source".into(),
        cursor: ProjectionCursor {
            generation: 2,
            memory_id: "z".into(),
        },
        complete: true,
        changes: vec![ProjectionChange {
            memory_id: "claim".into(),
            revision: 2,
            generation: 2,
            enabled: false,
            anchors: vec![],
        }],
    };
    graph
        .apply_memory_projection(&deletion, &batch.cursor)
        .await
        .unwrap();
    assert!(
        graph
            .memory_projection_candidates("source", &["nodes:root".into()])
            .await
            .unwrap()
            .is_empty()
    );
    let records: Vec<serde_json::Value> = storage
        .db()
        .query("SELECT * FROM memory_projection")
        .await
        .unwrap()
        .check()
        .unwrap()
        .take(0)
        .unwrap();
    assert!(records.is_empty());
}
