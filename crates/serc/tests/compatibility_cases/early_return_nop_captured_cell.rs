use crate::common;

#[test]
fn captured_return_after_if_join() {
    common::assert_matches_cpython_marshal(
        r"def run(resource, flag):
    def capture():
        return resource
    try:
        if flag:
            prepare(resource)
        consume(resource)
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)
",
        "captured_return_after_if_join.py",
    );
}

#[test]
fn captured_return_after_for_report() {
    common::assert_matches_cpython_marshal(
        r#"def run(resource, chunks, all_chunks):
    def capture():
        return resource
    try:
        for chunk in chunks:
            consume(chunk)
        print(json.dumps({"selected": len(chunks), "total": len(all_chunks)}))
        return 0
    finally:
        if resource is not None:
            resource.close()
"#,
        "captured_return_after_for_report.py",
    );
}

#[test]
fn matches_cpython_marshal_for_early_return_nop_local_control() {
    let source = r"def early_return_nop_owner(resource):
    try:
        consume(resource)
        consume(resource)
        consume(resource)
        return 0
    finally:
        if resource is not None:
            finish(resource)
";
    common::assert_matches_cpython_marshal(source, "early_return_nop_local_control.py");
}
