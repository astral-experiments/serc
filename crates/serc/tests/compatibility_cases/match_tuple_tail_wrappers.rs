use crate::common;

#[test]
fn matches_cpython_match_tuple_return_through_runtime_tail_wrappers() {
    for source in [
        "def f(subject, outer, optional, inner, example, comment):\n    match subject:\n        case Kind():\n            if outer:\n                if optional:\n                    if inner:\n                        comment = f\"{comment} yes\"\n                    else:\n                        comment = \"no\"\n            else:\n                comment = \"other\"\n                if optional:\n                    comment = f\"{comment} tail\"\n        case _:\n            comment = \"default\"\n    return example, comment\n",
        "def f(subject, outer, optional, inner, example, comment):\n    match subject:\n        case Kind():\n            if outer:\n                if optional:\n                    if subject:\n                        if inner:\n                            comment = f\"{comment} yes\"\n                        else:\n                            comment = \"no\"\n            else:\n                comment = \"other\"\n                if optional:\n                    comment = f\"{comment} tail\"\n        case _:\n            comment = \"default\"\n    return example, comment\n",
    ] {
        common::assert_matches_cpython_marshal(source, "match_tuple_tail_wrappers.py");
    }
}

#[test]
fn matches_cpython_match_tuple_return_with_independent_case_sources() {
    let prefix = "def f(subject, outer, optional, inner, example, comment, values, default):\n    match subject:\n        case Kind():\n            if outer:\n                if optional:\n                    if inner:\n                        comment = f\"{comment} yes\"\n                    else:\n                        comment = \"no\"\n            else:\n                comment = \"other\"\n                if optional:\n                    comment = f\"{comment} tail\"\n";
    let suffix =
        "        case _:\n            comment = \"default\"\n    return example, comment\n";
    for arm in [
        "        case {\"fields\": list(fields)}:\n            for field in fields:\n                example = field\n",
        "        case {\"type\": str(name), \"default\": value}:\n            example = value\n            comment = f\"type {name!r}\"\n",
        "        case object() if not default:\n            example = values.get(str(subject))\n            comment = f\"type {subject!r}\"\n",
    ] {
        common::assert_matches_cpython_marshal(
            &format!("{prefix}{arm}{suffix}"),
            "match_tuple_independent_case_sources.py",
        );
    }
}
