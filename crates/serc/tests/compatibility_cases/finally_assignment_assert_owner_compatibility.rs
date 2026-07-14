use crate::common;

#[test]
fn matches_cpython_marshal_for_finally_assignment_assert_owner() {
    let source = r#"class Tracker:
    def finish_batch(self):
        try:
            assert self.batch is not None
            if self.batch:
                widget = self.receiver()
                assert widget is not None
                widget.generate("changed", data=self.batch)
        finally:
            self.batch = None
"#;
    common::assert_matches_cpython_marshal(source, "finally_assignment_assert_owner.py");
}
