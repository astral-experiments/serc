use crate::common;

#[test]
fn matches_cpython_backward_jump_width_at_extended_argument_boundary() {
    for prefix_length in [0, 63, 64, 65, 96] {
        for body_length in [25, 26, 27] {
            for tail in ["", "        consume(value)\n"] {
                let prefix = "    value += 1\n".repeat(prefix_length);
                let body = "        value += 1\n".repeat(body_length);
                let source = format!(
                    "def f(value, flag):\n{prefix}    while flag:\n{body}{tail}    return value\n"
                );
                common::assert_matches_cpython_marshal(&source, "jump_argument_width_seeds.py");
            }
        }
    }
}
