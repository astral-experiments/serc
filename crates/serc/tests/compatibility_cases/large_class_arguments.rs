use crate::common;

#[test]
fn matches_cpython_large_class_call_arguments() {
    for base_count in [30, 31, 48] {
        for keyword_count in [0, 15, 16] {
            for parameters in ["", "[T]"] {
                for nested in [false, true] {
                    let mut arguments = (0..base_count)
                        .map(|index| format!("bases.B{index}"))
                        .collect::<Vec<_>>();
                    arguments
                        .extend((0..keyword_count).map(|index| format!("keyword{index}=value")));
                    let source = format!(
                        "class Example{parameters}(\n    {}\n):\n    pass\n",
                        arguments.join(",\n    "),
                    );
                    let source = if nested {
                        format!(
                            "def build(bases, value):\n    {}\n    return Example\n",
                            source.trim_end().replace('\n', "\n    "),
                        )
                    } else {
                        source
                    };
                    common::assert_matches_cpython_marshal(&source, "large_class_arguments.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_large_class_unpacking_arguments() {
    for base_count in [27, 28, 29, 30] {
        for first in [false, true] {
            for parameters in ["", "[T]"] {
                for keyword_unpack in [false, true] {
                    let mut arguments = (0..base_count)
                        .map(|index| format!("bases.B{index}"))
                        .collect::<Vec<_>>();
                    arguments.insert(if first { 0 } else { base_count }, "*items".into());
                    if keyword_unpack {
                        arguments.push("**options".into());
                        arguments.extend((0..16).map(|index| format!("keyword{index}=value")));
                    }
                    let source = format!(
                        "class Example{parameters}({}):\n    pass\n",
                        arguments.join(", "),
                    );
                    common::assert_matches_cpython_marshal(&source, "large_class_unpacking.py");
                }
            }
        }
    }
}
