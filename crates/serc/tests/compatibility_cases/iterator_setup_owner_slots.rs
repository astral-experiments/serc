use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_iterator_setup_owner_slots() {
    for prefix_calls in [0, 8, 9, 12] {
        let mut prefix = String::new();
        for index in 0..prefix_calls {
            writeln!(prefix, "        consume(item, {index})").unwrap();
        }
        for body in [
            "try:\n    value = item.make()\n    if value.flags.owned:\n        continue\n    value = value.view(Type)",
            "try: value = item.make(); result = value if value.flags.owned else None",
        ] {
            let source = format!(
                "def f(items):\n    for item in items:\n{prefix}        {}\n        except Error:\n            continue\n        use(value)\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "iterator_setup_owner_slots.py");
        }
    }
}
