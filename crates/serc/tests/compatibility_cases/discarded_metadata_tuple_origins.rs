use crate::common;

#[test]
fn matches_cpython_discarded_metadata_tuple_origins() {
    for seed in [
        "",
        "def seed():\n    return café\n",
        "if False:\n    def seed():\n        return café\n",
        "if False:\n    def seed(café):\n        pass\n",
        "def discarded():\n    return None\n    def seed():\n        return café\n",
    ] {
        let source =
            format!("scalar = 'café'\n{seed}values = ('café',)\nprint(scalar is values[0])\n");
        common::assert_matches_cpython_marshal(&source, "discarded_metadata_tuple_origins.py");
        common::assert_executes(&source, if seed.is_empty() { "True\n" } else { "False\n" });
    }
}

#[test]
fn discarded_annotation_locals_do_not_intern_equal_literal_tuples() {
    let source = "if False:\n    def discarded(value: [café for café in ()]): pass\nscalar = 'café'\nvalues = ('format', 'café')\nprint(scalar is values[1])\n";
    common::assert_matches_cpython_marshal(source, "discarded_annotation_locals.py");
    common::assert_executes(source, "True\n");
}
