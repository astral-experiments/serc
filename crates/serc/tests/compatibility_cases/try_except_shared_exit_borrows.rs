use crate::common;

#[test]
fn matches_cpython_borrows_after_shared_conditional_try_exits() {
    for prefix in ["", "async "] {
        for condition in ["flag", "flag and value", "flag or value"] {
            for returned in ["value", "value.attr"] {
                let source = format!(
                    "{prefix}def probe(values, flag, work, use):\n    for item in values:\n        try:\n            value = work(item)\n            if {condition}:\n                return {returned}\n        except Exception:\n            pass\n        use(item)\n"
                );
                common::assert_matches_cpython_marshal(&source, "shared_conditional_try_exit.py");
            }
        }
    }
}
