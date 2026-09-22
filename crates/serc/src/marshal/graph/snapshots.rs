use std::fmt::Write as _;

use super::ObjectGraph;

#[test]
fn shared_constant_and_metadata_identities() {
    let code = crate::compile(
        "value = ('shared', 'shared')\ndef f(arg):\n    return ('shared', 'shared'), arg\n",
        "identities.py",
    )
    .unwrap();
    let graph = ObjectGraph::for_code(&code.code);
    let mut snapshot = String::new();
    for (index, object) in graph.objects_by_id.iter().enumerate() {
        writeln!(
            snapshot,
            "object {index}: shared={} cached={} interned={}",
            object.shared, object.cached, object.interned
        )
        .unwrap();
    }
    for (index, occurrence) in graph.occurrences.iter().enumerate() {
        writeln!(
            snapshot,
            "visit {index}: {:?} -> object {}",
            occurrence.kind,
            occurrence.object.index()
        )
        .unwrap();
    }
    insta::assert_snapshot!(snapshot);
}
