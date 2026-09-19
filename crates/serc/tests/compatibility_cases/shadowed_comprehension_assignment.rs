use crate::common;

#[test]
fn matches_cpython_shadowed_comprehension_assignment_restores() {
    for expression in [
        "[data for data in data if data]",
        "{data for data in data if data}",
        "{data: data for data in data if data}",
        "[item for data in data for item in data]",
        "[data for data in values if data]",
        "[item for item in data if item]",
    ] {
        for definition in ["def", "async def"] {
            for protected in [false, true] {
                let source = if protected {
                    format!(
                        "{definition} function(data, values):\n    try:\n        data = {expression}\n    except Exception:\n        pass\n    return data\n"
                    )
                } else {
                    format!(
                        "{definition} function(data, values):\n    data = {expression}\n    return data\n"
                    )
                };
                common::assert_matches_cpython_marshal(&source, "comprehension_assignment.py");
            }
        }
    }
}

#[test]
fn matches_cpython_repeated_ordinary_assignment_stores() {
    for target in ["data, data", "data, other, data"] {
        for protected in [false, true] {
            let source = if protected {
                format!(
                    "def function(values):\n    try:\n        {target} = values\n    except Exception:\n        pass\n    return data\n"
                )
            } else {
                format!("def function(values):\n    {target} = values\n    return data\n")
            };
            common::assert_matches_cpython_marshal(&source, "ordinary_assignment.py");
        }
    }
}
