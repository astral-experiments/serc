#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_filtered_comprehension_none_condition_owners() {
    let source = r#"
def datadog_element_after_filter(fields, lookup):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        (
            any(extra for extra in details.values),
            {"v": details.value, **({"x": lookup(name)} if name is not None else {})},
        )
        for name, details in fields
        if details.ready
    ]


def optuna_eager_filter_argument(fields, lookup):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        details.value
        for name, details in fields
        if lookup(lookup(name) if name is not None else details.value)
    ]


def direct_filter_negative(fields, lookup):
    return [details.value for name, details in fields if lookup(name) if name is not None]


def exact_negative(fields, lookup):
    return [
        lookup(name) if name is not None else details.value
        for name, details in fields
        if details.ready
    ]
"#;

    common::assert_matches_cpython_marshal(
        source,
        "filtered_comprehension_none_condition_owners.py",
    );
}
