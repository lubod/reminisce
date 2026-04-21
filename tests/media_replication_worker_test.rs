/// Tests for media_replication_worker — focused on the pure rendezvous hashing
/// function which is the critical data-placement algorithm.
use reminisce::media_replication_worker::{rendezvous_select_nodes, SHARD_COUNT, MIN_NODES_REQUIRED};
use std::net::SocketAddr;

fn make_nodes(n: usize) -> Vec<(String, SocketAddr)> {
    (0..n).map(|i| {
        let addr: SocketAddr = format!("127.0.0.1:{}", 9000 + i).parse().unwrap();
        (format!("node_{:04}", i), addr)
    }).collect()
}

// ---------------------------------------------------------------------------
// rendezvous_select_nodes
// ---------------------------------------------------------------------------

#[test]
fn test_rendezvous_returns_correct_count() {
    let nodes = make_nodes(10);
    let selected = rendezvous_select_nodes("file_abc", &nodes, 5);
    assert_eq!(selected.len(), 5);
}

#[test]
fn test_rendezvous_capped_at_available_nodes() {
    let nodes = make_nodes(3);
    // Request 5 but only 3 available — must not panic, must return 3.
    let selected = rendezvous_select_nodes("file_abc", &nodes, 5);
    assert_eq!(selected.len(), 3);
}

#[test]
fn test_rendezvous_empty_nodes_returns_empty() {
    let nodes: Vec<(String, SocketAddr)> = vec![];
    let selected = rendezvous_select_nodes("file_abc", &nodes, 5);
    assert!(selected.is_empty());
}

#[test]
fn test_rendezvous_is_stable_for_same_file() {
    let nodes = make_nodes(10);
    let a = rendezvous_select_nodes("my_file_hash", &nodes, 5);
    let b = rendezvous_select_nodes("my_file_hash", &nodes, 5);
    // Same file + same node list → same result every time
    let ids_a: Vec<&str> = a.iter().map(|(id, _)| id.as_str()).collect();
    let ids_b: Vec<&str> = b.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids_a, ids_b);
}

#[test]
fn test_rendezvous_different_files_get_different_placements() {
    let nodes = make_nodes(10);
    let a = rendezvous_select_nodes("file_aaa", &nodes, 5);
    let b = rendezvous_select_nodes("file_bbb", &nodes, 5);
    let ids_a: Vec<&str> = a.iter().map(|(id, _)| id.as_str()).collect();
    let ids_b: Vec<&str> = b.iter().map(|(id, _)| id.as_str()).collect();
    // Very unlikely to be identical for different file hashes
    assert_ne!(ids_a, ids_b, "Two different files should generally map to different nodes");
}

#[test]
fn test_rendezvous_results_are_subset_of_input() {
    let nodes = make_nodes(8);
    let input_ids: std::collections::HashSet<String> = nodes.iter().map(|(id, _)| id.clone()).collect();
    let selected = rendezvous_select_nodes("test_file", &nodes, 5);
    for (id, _) in &selected {
        assert!(input_ids.contains(id), "Selected node {} was not in input", id);
    }
}

#[test]
fn test_rendezvous_no_duplicate_nodes() {
    let nodes = make_nodes(10);
    let selected = rendezvous_select_nodes("test_dedup", &nodes, 5);
    let mut seen = std::collections::HashSet::new();
    for (id, _) in &selected {
        assert!(seen.insert(id), "Node {} selected twice", id);
    }
}

#[test]
fn test_rendezvous_adding_node_minimises_remapping() {
    // Rendezvous/HRW has the minimum disruption property: adding one node
    // should only move ~1/N of the shards. Test that most assignments are
    // stable when the topology grows by one.
    let nodes5 = make_nodes(5);
    let mut nodes6 = nodes5.clone();
    nodes6.push(("node_0005".to_string(), "127.0.0.1:9005".parse().unwrap()));

    let file_hashes: Vec<&str> = vec![
        "aaaa", "bbbb", "cccc", "dddd", "eeee",
        "ffff", "gggg", "hhhh", "iiii", "jjjj",
    ];

    let mut total_checks = 0usize;
    let mut stable = 0usize;

    for hash in &file_hashes {
        let old = rendezvous_select_nodes(hash, &nodes5, 3);
        let new = rendezvous_select_nodes(hash, &nodes6, 3);
        let old_ids: std::collections::HashSet<&str> = old.iter().map(|(id, _)| id.as_str()).collect();
        let new_ids: std::collections::HashSet<&str> = new.iter().map(|(id, _)| id.as_str()).collect();
        stable += old_ids.intersection(&new_ids).count();
        total_checks += 3;
    }

    // At least 60% of assignments should be stable (theoretical ideal is ~83% for 5→6 nodes)
    assert!(
        stable * 100 / total_checks >= 60,
        "Only {}% stable assignments after adding a node (expected ≥60%)",
        stable * 100 / total_checks
    );
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn test_shard_count_is_five() {
    assert_eq!(SHARD_COUNT, 5, "3/5 erasure coding requires exactly 5 total shards");
}

#[test]
fn test_min_nodes_required_documented() {
    // MIN_NODES_REQUIRED = 1 allows replication even with a single node.
    // NOTE: With a single node, all 5 shards land on one device and EC provides
    // no fault tolerance. Recommend deploying with ≥3 nodes for real safety.
    assert!(MIN_NODES_REQUIRED >= 1);
}
