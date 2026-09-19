use crate::common;

#[test]
fn matches_cpython_tuple_annotation_strings() {
    for annotation in ["(A, B)", "(A,)", "()", "((A, B), C)", "tuple[(A, B)]"] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: {annotation}) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "tuple_annotation_strings.py");
    }
}

#[test]
fn matches_cpython_concatenated_fstring_annotation_strings() {
    for value in [
        "'prefix ' f'{value}'",
        "f'{value}' ' suffix'",
        "f'{left}' f'{right}'",
        "'a' f'b{value}' 'c' f'd{other}'",
        "r'\\path ' fr'{value}\\x'",
        "f'{{braces}}' f'{value}'",
        "'prefix ' f'{value!r:>{width}}'",
        "f'{value}' '\\nend'",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, {value}]):\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "concatenated_fstring_annotations.py");
    }
}

#[test]
fn matches_cpython_fstring_annotation_body_repr() {
    for annotation in [
        r#"f"single' {value}""#,
        r#"f'double" {value}'"#,
        r#"f"both'\" {value}""#,
        r#"'single quote: ' f"'{value}""#,
        r"f'line\n{value}\tend'",
        r"rf'path\n{value}'",
        r"f'{{braces}} {value}'",
        r#"f"{'quoted'}""#,
        r#"f"{f"nested' {value}"}""#,
        r"f'{value if flag else other}'",
        r"f'{(lambda value: value)}'",
        r"f'{(left, right)}'",
        r#"f"{{literal}} { {'key': value} }""#,
    ] {
        let source = format!(
            "from __future__ import annotations\nmodule: {annotation}\nclass C:\n    field: {annotation}\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "fstring_annotation_body_repr.py");
    }
}

#[test]
fn matches_cpython_debug_fstring_annotation_strings() {
    for annotation in [
        r"f'{ value  = }'",
        r"f'{value +  other = }'",
        r"f'{(value + other)=}'",
        r"f'{value = !s}'",
        r"f'{value = :>10}'",
        r"f'{value = :{width}}'",
        r"f'{value!a}'",
        r"f'{value!r:{width}.{precision}f}'",
        r#"f"{'quoted'=}""#,
        r"'a' f'{value=}' 'b' f'{other}'",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "debug_fstring_annotation_strings.py");
    }
}

#[test]
fn matches_cpython_annotation_call_argument_order() {
    for annotation in [
        "func(key=value, *args)",
        "func(*first, key=value, *second)",
        "func(key=value, *args, **mapping)",
        "func(a=first, *left, b=second, *right, **more)",
        "func(key=nested(other=value, *args), *args)",
        "func(key=f'keyword{value}', *f'argument{other}')",
        "func(key=u'keyword', *u'argument')",
        "func(*args, key=value)",
        "func(value, key=value)",
        "func(**mapping)",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "annotation_call_argument_order.py");
    }
}

#[test]
fn preserves_tstring_annotation_interpolation_source() {
    for annotation in [
        "t'{func(key=value, *args)}'",
        "t'{func(key=value, *left, other=other, *right)}'",
        "t'{func(key=value, *args)!r}'",
        "t'{func(key=value, *args):{width}}'",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "tstring_annotation_interpolation.py");
    }
}

#[test]
fn matches_cpython_unpacked_annotation_keyword_precedence() {
    for value in [
        "left | right",
        "left + right",
        "left ** right",
        "-value",
        "not value",
        "left and right",
        "left < right",
        "value if flag else other",
        "lambda: value",
        "(left, right)",
        "(value for value in values)",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, factory(**({value}))]):\n    pass\n"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "unpacked_annotation_keyword_precedence.py",
        );
    }
}

#[test]
fn matches_cpython_variadic_lambda_annotation_spacing() {
    for parameters in [
        "*args",
        "**kwargs",
        "*, value",
        "*args, value=other, **kw",
        "value, *args, **kw",
        "value, /, other=default",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, (lambda {parameters}: value)]):\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "variadic_lambda_annotation_spacing.py");
    }
}

#[test]
fn matches_cpython_template_annotation_strings() {
    for annotation in [
        "t'{(lambda *args: args)}'",
        "t'{ left  + right }'",
        "t'{func(key=value, *args)}'",
        "t'{value=}'",
        "t'{ value = !s}'",
        "t'{value = :>{ width +  other }}'",
        "t'left {value}' t' right {other}'",
        r"rt'path\n{value}'",
        "t'''quote' {value}'''",
        "t'''line\n{value}'''",
        r#"t"{'quoted'=}""#,
        "t'{value!r:{(lambda *args: args)}}'",
    ] {
        let source = format!(
            "from __future__ import annotations\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "template_annotation_strings.py");
    }
}

#[test]
fn matches_cpython_interpolation_comment_metadata() {
    for prefix in ['f', 't'] {
        for expression in [
            "(value # ordinary\n)",
            "(value # first\n + other # second\n)",
            "('hash#quoted' # outside\n)",
            "('escaped\\'quote#inner' # outside\n)",
        ] {
            let annotation = format!("{prefix}\"\"\"{{{expression}=}}\"\"\"");
            let source = format!(
                "from __future__ import annotations\ndef f(value: Annotated[A, {annotation}]) -> {annotation}:\n    pass\n"
            );
            common::assert_matches_cpython_marshal(&source, "interpolation_comment_metadata.py");
        }
    }
}
