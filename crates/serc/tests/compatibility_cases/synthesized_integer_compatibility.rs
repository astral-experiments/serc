use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_marshal_for_wide_sequence_pattern_integers() {
    for size in [255, 256] {
        let patterns = [
            vec!["_"; size],
            [vec!["_"; size], vec!["captured", "*_"]].concat(),
            [vec!["*_", "captured"], vec!["_"; size - 1]].concat(),
        ];
        for (index, pattern) in patterns.into_iter().enumerate() {
            for prefix in ["", "seed = 'existing constant'\n"] {
                let source = format!(
                    "{prefix}match subject:\n    case [{}]:\n        pass\n",
                    pattern.join(", ")
                );
                assert_matches_cpython_marshal(
                    &source,
                    &format!("sequence_pattern_{size}_{index}.py"),
                );
            }
        }
    }
}

#[test]
fn matches_cpython_marshal_for_wide_relative_import_levels() {
    for level in [255, 256] {
        for prefix in ["", "seed = 'existing constant'\n"] {
            let source = format!("{prefix}from {}module import value\n", ".".repeat(level));
            assert_matches_cpython_marshal(&source, &format!("relative_import_{level}.py"));
        }
    }
}
