use crate::common;

#[test]
fn preserves_comprehension_body_references_before_later_iteration_targets() {
    for expression in [
        "[lookup(value) for value in values]",
        "{lookup(value): value for value in values}",
        "[value for value in values if lookup(value)]",
        "[other for value in values for other in lookup(value)]",
        "[[lookup(other) for other in value] for value in values]",
        "[(lambda result=lookup(value): result) for value in values]",
    ] {
        for (parameters, declaration, captures_outer) in [
            ("values", "", false),
            ("values, lookup", "", false),
            ("values", "    lookup = make_lookup()\n", false),
            ("values", "", true),
        ] {
            let function = format!(
                "def function({parameters}):\n{declaration}    result = {expression}\n    shadowed = [lookup for lookup in values]\n    return result, shadowed\n"
            );
            let source = if captures_outer {
                let indented = function.trim_end().replace('\n', "\n    ");
                format!("def enclosing(lookup):\n    {indented}\n    return function\n")
            } else {
                function
            };
            common::assert_matches_cpython_marshal(&source, "comprehension_symbol_scope_order.py");
        }
    }
}

#[test]
fn preserves_earlier_comprehension_bindings_and_outer_references() {
    for body in [
        "shadowed = [lookup for lookup in values]\nresult = [lookup(value) for value in values]",
        "shadowed = [lookup for lookup in values]\nresult = [value for value in values if lookup(value)]",
        "result = [lookup(value) for lookup in callbacks for value in values]\nshadowed = [lookup for lookup in values]",
        "result = [[lookup(value) for value in values] for lookup in callbacks]\nshadowed = [lookup for lookup in values]",
        "original = lookup\nshadowed = [lookup for lookup in values]\nresult = [lookup(value) for value in values]",
        "result = [lookup for lookup in lookup(values)]\nshadowed = [lookup for lookup in values]",
    ] {
        let indented = body.replace('\n', "\n        ");
        let source = format!(
            "def enclosing(lookup):\n    def function(values, callbacks):\n        {indented}\n        return result, shadowed\n    return function\n"
        );
        common::assert_matches_cpython_marshal(&source, "comprehension_symbol_scope_order.py");
    }
}

#[test]
fn preserves_captured_comprehension_targets_in_free_variable_slots() {
    for target in [
        "[(lambda: lookup) for lookup in values]",
        "{lookup: (lambda: lookup) for lookup in values}",
        "[[(lambda: lookup) for lookup in values] for item in values]",
        "[[(lambda: lookup) for lookup in lookup(values)] for item in values]",
    ] {
        for declaration in ["", "        nonlocal lookup\n"] {
            let source = format!(
                "def enclosing(lookup):\n    def function(values):\n{declaration}        result = [lookup(value) for value in values]\n        target = {target}\n        return result, target\n    return function\n"
            );
            common::assert_matches_cpython_marshal(&source, "comprehension_free_cell.py");
        }
    }
}

#[test]
fn preserves_inlined_symbol_admission_and_empty_save_scopes() {
    for source in [
        // own reference after child
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [([(lambda: probe) for probe in values], probe) for outer in values]
        return result
    return function
",
        // lambda default own reference
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [((lambda value=probe: value), [(lambda: probe) for probe in values]) for outer in values]
        return result
    return function
",
        // named target before child
        r"def enclosing(probe):
    def function(values):
        nonlocal probe
        prelude = [probe(value) for value in values]
        result = [((probe := item), [(lambda: probe) for probe in values]) for item in values]
        return result
    return function
",
        // named target after child
        r"def enclosing(probe):
    def function(values):
        nonlocal probe
        prelude = [probe(value) for value in values]
        result = [([(lambda: probe) for probe in values], (probe := item)) for item in values]
        return result
    return function
",
        // implicit __class__ from super
        r"class Example:
    def function(self, values):
        ordinary = __class__
        result = [(super(), [lambda: __class__ for __class__ in values]) for item in values]
        return result
",
        // ordinary dictionary key-first admission
        r"def enclosing(probe):
    def function(values):
        ordinary = probe
        x = None
        result = [{0: [(lambda: probe) for probe in values], tuple([probe(x) for x in values]): 0} for item in values]
        return result
    return function
",
        // bound child wins before closure free
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [([probe for probe in values], [(lambda: probe) for x in values]) for outer in values]
        return result
    return function
",
        // conditional first iterable
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[(lambda: probe) for probe in (probe(values) if flag else values)] for outer in values]
        return result
    return function
",
        // intermediate bound target masks deep reference
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[[(lambda: probe) for probe in probe(values)] for probe in values] for outer in values]
        return result
    return function
",
        // later iterable remains current-scope reference
        r"def enclosing(probe):
    def function(values, flag):
        ordinary = probe
        result = [[(lambda: probe) for dummy in values for probe in probe(values)] for outer in values]
        return result
    return function
",
        // target receiver forces zero-save free
        r"def enclosing(probe):
    def function(values):
        result = [[lambda: probe for probe in values] for probe.attr in values]
        return result
    return function
",
        // target subscript forces zero-save free
        r"def enclosing(probe):
    def function(values):
        result = [[lambda: probe for probe in values] for probe[0] in values]
        return result
    return function
",
        // later target traversal before iterable
        r"def enclosing(probe):
    def function(values, holder):
        ordinary = probe
        result = [None for item in values for holder[[probe(x) for x in values]] in [(lambda: probe) for probe in values]]
        return result
    return function
",
        // class global load in zero-save scope
        r"class C:
    result = [foo() for obj.attr in values]
",
        // module nested lambda flag in zero-save scope
        r"result = [lambda: 0 for obj.attr in values]
",
        // module generator flag in zero-save scope
        r"result = [(foo for item in values) for obj.attr in values]
",
    ] {
        common::assert_matches_cpython_marshal(source, "inlined_symbol_admission.py");
    }
}

#[test]
fn preserves_symbol_admission_order_across_expression_and_statement_fields() {
    for source in [
        r"def enclosing(lookup):
    def function(values):
        result = {0: [lookup(v) for v in values], tuple([lookup for lookup in values]): 0}
        return result
    return function
",
        r"def enclosing(lookup):
    def function(values, holder):
        result = {tuple([lookup(v) for v in values]): [lookup for lookup in values] for item in values}
        return result
    return function
",
        r"def enclosing(lookup):
    def function(values, holder):
        result = [0 for item in values for holder[tuple([lookup for lookup in values])] in [lookup(v) for v in values]]
        return result
    return function
",
        r"def enclosing(lookup):
    def function(values):
        @[lookup(v) for v in values]
        def child(arg=[lookup for lookup in values]):
            return arg
        return child
    return function
",
        r"def enclosing(lookup):
    def function(values):
        @[lookup(v) for v in values]
        async def child(arg=[lookup for lookup in values]):
            return arg
        return child
    return function
",
        r"def enclosing(lookup):
    def function(values, holder):
        holder[tuple([lookup for lookup in values])] = [lookup(v) for v in values]
        return None
    return function
",
        r"def enclosing(lookup):
    def function(values, holder):
        holder[tuple([lookup for lookup in values])] += [lookup(v) for v in values]
        return None
    return function
",
        r"def enclosing(lookup):
    def function(values, holder):
        for holder[tuple([lookup for lookup in values])] in [lookup(v) for v in values]:
            pass
        return None
    return function
",
    ] {
        common::assert_matches_cpython_marshal(source, "comprehension_scope_field_order.py");
    }
}

#[test]
fn reserves_deferred_nonlocal_targets_at_their_bound_comprehension() {
    for source in [
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([lookup for lookup in values], [x for x in lookup(values)]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([x for x in lookup(values)], [lookup for lookup in values]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([(lambda: lookup) for lookup in values], [x for x in lookup(values)]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([x for x in lookup(values)], [(lambda: lookup) for lookup in values]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([lookup for lookup in values], [other for other in values], [x for x in lookup(values)], [y for y in other(values)]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([x for x in lookup(values)], [y for y in other(values)], [lookup for lookup in values], [other for other in values]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([(lambda: lookup) for lookup in values], [(lambda: other) for other in values], [x for x in lookup(values)], [y for y in other(values)]) for item in values]
        return result
    return function
",
        r"def enclosing(lookup, other):
    def function(values):
        nonlocal lookup, other
        result = [([(lambda: lookup) for lookup in values], [other for other in values], [x for x in lookup(values)], [y for y in other(values)]) for item in values]
        return result
    return function
",
    ] {
        common::assert_matches_cpython_marshal(source, "comprehension_nonlocal_reservation.py");
    }
}

#[test]
fn reserves_nested_first_iterable_nonlocals_at_their_actual_save_scope() {
    common::assert_matches_cpython_marshal(
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([a for a in values], [b for b in values], a) for middle in values]], b) for root in values]
        return result
    return function
",
        "comprehension_first_iterable_reservation.py",
    );
}

#[test]
fn preserves_nonlocal_declarations_across_masked_first_iterable_scopes() {
    for source in [
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([a for a in values], [b for b in values], a) for middle in values]], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values]], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in identity([([a for a in values], [b for b in values], a) for middle in values])], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in identity([([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values])], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in {0: [([a for a in values], [b for b in values], a) for middle in values]}[0]], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in {0: [([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values]}[0]], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in (lambda arg=[([a for a in values], [b for b in values], a) for middle in values]: arg)()], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in (lambda arg=[([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values]: arg)()], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([a for a in values], [b for b in values], a) for middle in values]], b) for b in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([(lambda: a) for a in values], [(lambda: b) for b in values], a) for middle in values]], b) for b in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([a for a in values], [b for b in values], a) for a in values]], b) for root in values]
        return result
    return function
",
        r"def enclosing(a, b):
    def function(values):
        nonlocal a, b
        result = [([value for value in [([(lambda: a) for a in values], [(lambda: b) for b in values], a) for a in values]], b) for root in values]
        return result
    return function
",
    ] {
        common::assert_matches_cpython_marshal(source, "comprehension_nonlocal_first_iterable.py");
    }
}
