use crate::common;

#[test]
fn matches_cpython_marshal_for_mapping_key_tuple_folding_boundary() {
    for count in [29, 30, 31, 32] {
        for key_kind in ["integer", "string", "mixed"] {
            for rest in ["", ", **rest"] {
                let keys = (0..count)
                    .map(|index| {
                        let key = match key_kind {
                            "integer" => index.to_string(),
                            "string" => format!("\"key{index}\""),
                            _ if index == count / 2 => "Keys.value".to_string(),
                            _ => format!("\"key{index}\""),
                        };
                        format!("{key}: _")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let source = format!(
                    "def match_mapping(subject):\n    match subject:\n        case {{{keys}{rest}}}:\n            return True\n    return False\n"
                );
                let filename = format!("mapping_keys_{key_kind}_{count}_{}.py", !rest.is_empty());
                common::assert_matches_cpython_marshal(&source, &filename);
            }
        }
    }
}
