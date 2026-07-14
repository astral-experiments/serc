use crate::common;

#[test]
fn matches_cpython_marshal_for_filtered_comprehension_truth_condition_owners() {
    let source = r#"
def filtered_call_truthiness(values, omit, mapped):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [" " if mapped(value) else value for value in values if not omit(value)]


def filtered_subscript_truthiness(rows, styles, hidden):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        {**cell, "display": cell["value"] if cell["visible"] else "", "style": styles[index]}
        for index, cell in enumerate(rows)
        if cell["type"] == "header" and not hidden[index]
    ]


def filtered_name_truthiness(values, resolve, convert):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return {
        convert(name) if name else None
        for value in values
        if (name := resolve(value))
    }


def filtered_adjacent_truth_and_none(configs, methods):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        (
            f"{method}{'_buffered' if buffered else ''}{'_async' if async_mode else ''}"
            f"_{payload}_tracing_{'enabled' if tracing is None else 'disabled'}"
            f"{'_links_enabled' if payload == 'batch' and links is not None else ('_links_disabled' if payload == 'batch' else '')}",
            {
                "METHOD": method,
                **({"TRACING": str(tracing)} if tracing is not None else {}),
                **({"LINKS": str(links)} if links is not None else {}),
            },
        )
        for method, buffered, async_mode, payload, tracing, links in configs
        if method in methods
    ]


def nested_and_negated_truthiness(values, ready, choose):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        f"{'x' if not value else ''}{'yes' if ready and choose(value) and value is None else 'no'}"
        for value in values
        if ready
    ]


def middle_none_followed_by_truthiness(values, ready, holder):
    l00 = l01 = l02 = l03 = l04 = l05 = l06 = l07 = l08 = l09 = l10 = l11 = l12 = l13 = l14 = l15 = l16 = l17 = l18 = l19 = l20 = l21 = l22 = l23 = None
    return [
        f"{'yes' if ready and value is None and holder.ready else 'no'}"
        for value in values
        if ready
    ]


def nested_inlined_comprehension_prefix(values, ready, extra):
    return [
        f"{[item if value else ready for item in extra]}{'yes' if value else 'no'}"
        for value in values
        if ready
    ]


def filtered_dict_comprehension_key(values, ready, required, optional):
    return {
        (name, required(value) if value.flag else optional(value)): value
        for name, value in values
        if ready
    }
"#;

    common::assert_matches_cpython_marshal(
        source,
        "filtered_comprehension_truth_condition_owners.py",
    );
}
