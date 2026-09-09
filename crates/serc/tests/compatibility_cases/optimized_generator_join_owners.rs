use crate::common;

#[test]
fn matches_cpython_exception_owners_after_optimized_generator_call_joins() {
    for callable in ["any", "all", "tuple"] {
        for expression in ["{call}", "flag and {call}", "{call} or flag"] {
            for action in ["continue", "break", "use(value)", "return value"] {
                let call = format!("{callable}(item.flag for item in value.items)");
                let condition = expression.replace("{call}", &call);
                let source = format!(
                    "def f(manager, values, flag):\n    with manager:\n        for value in values:\n            if {condition}:\n                {action}\n            after(value)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "optimized_generator_join_owners.py",
                );
            }
        }
    }
}
