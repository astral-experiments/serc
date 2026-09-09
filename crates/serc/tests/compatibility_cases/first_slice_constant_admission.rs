use crate::common;

#[test]
fn first_slice_constant_survives_unreachable_code() {
    for declaration in ["def", "async def"] {
        for expression in [
            "value[:]",
            "value[1:]",
            "value[:stop]",
            "value[start:]",
            "value[start:stop]",
            "value[0:stop]",
            "value[:, :stop]",
            "value[start, :]",
            "value[::False]",
            "value[1 + 1:]",
        ] {
            for body in [
                format!("return value\nresult = {expression}\nconsume(0)"),
                format!("raise Error\ndel {expression}\nconsume(0)"),
                format!("return value\n{expression} = start\nconsume(0)"),
            ] {
                let source = format!(
                    "{declaration} f(value, start, stop):\n    {}\n",
                    body.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "first_slice_constant.py");
            }
        }
    }
}

#[test]
fn slice_admission_preserves_earlier_constants_and_live_nested_scopes() {
    for declaration in ["def", "async def"] {
        for expression in [
            "value[:, :stop]",
            "value[start:]",
            "value[(lambda: start)():]",
        ] {
            for body in [
                format!(
                    "\"\"\"Existing docstring.\"\"\"\nreturn value\nresult = {expression}\nconsume(0)"
                ),
                format!("consume(1234)\nreturn value\nresult = {expression}\nconsume(0)"),
                format!("return None\nresult = {expression}\nconsume(0)"),
                format!("result = {expression}\nconsume(0)\nreturn result"),
            ] {
                let source = format!(
                    "{declaration} f(value, start, stop):\n    {}\n",
                    body.replace('\n', "\n    "),
                );
                common::assert_matches_cpython_marshal(&source, "slice_constant_order.py");
            }
        }
    }
}

#[test]
fn annotation_only_slices_admit_only_evaluated_bounds() {
    for future in ["", "from __future__ import annotations\n"] {
        for declaration in ["def", "async def"] {
            for annotation in [
                "value[:]: object",
                "value[1:None]: object",
                "value[True:False:0]: object",
                "value[stop:]: object",
                "value[other[:]:]: object",
                "value[:, other[:]]: object",
                "result: list[value[:]]",
                "value.attribute: list[other[:]]",
            ] {
                let source = format!(
                    "{future}{declaration} f(value, stop, other):\n    return value\n    {annotation}\n    consume(0)\n",
                );
                common::assert_matches_cpython_marshal(&source, "annotation_slice_admission.py");
            }
        }
    }
}
