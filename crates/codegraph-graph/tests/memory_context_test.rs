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
