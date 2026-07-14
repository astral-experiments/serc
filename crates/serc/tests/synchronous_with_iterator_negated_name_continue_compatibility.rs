#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_iterator_negated_name_continue() {
    let source = r"def apprise_style(manager, filenames, module_re, use):
    with manager:
        for filename in filenames:
            match = module_re.match(filename)
            if not match:
                continue
            use(match)


def dulwich_style(manager, lines, use):
    with manager:
        for line in lines:
            sha = line.strip()
            if not sha:
                continue
            use(sha)


def jax_style(manager, lines, use):
    with manager:
        for line in lines:
            stripped_line = line.strip()
            if not stripped_line:
                continue
            use(stripped_line)


def sklearn_style(manager, lines, pattern, use):
    with manager:
        for line in lines:
            matched = pattern.match(line)
            if not matched:
                continue
            use(matched)


def spack_style(manager, groups, environment, use):
    with manager:
        for group in groups:
            group_specs = environment.user_specs_by(group=group)
            if not group_specs:
                continue
            use(group_specs)


def strawberry_style(open_output, lines, use):
    with open_output() as output:
        for raw_line in lines:
            line = raw_line.strip()
            if not line:
                continue
            use(output, line)


def streamlit_style(manager, lines, parse, use):
    with manager:
        for line in lines:
            stripped = line.strip()
            if not stripped:
                continue
            entry = parse(stripped)
            use(entry)


def outer_marker(manager, lines, parse, use, flag):
    if flag:
        use(flag)
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


def same_with_marker(manager, lines, parse, use, flag):
    with manager:
        if flag:
            use(flag)
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


def multi_with(manager, other, lines, parse, use):
    with manager, other:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


def nested_with(manager, other, lines, parse, use):
    with manager:
        with other:
            for line in lines:
                value = parse(line)
                if not value:
                    continue
                use(value)


def for_else(manager, lines, parse, use):
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)
        else:
            use(None)


def second_loop(manager, first_lines, lines, parse, use):
    with manager:
        for first in first_lines:
            use(first)
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


def outer_try(manager, lines, parse, use):
    try:
        with manager:
            for line in lines:
                value = parse(line)
                if not value:
                    continue
                use(value)
    except ValueError:
        use(None)


def no_following_control(manager, lines, parse):
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue


def prior_for_control(manager, groups, lines, parse, use):
    with manager:
        for line in lines:
            for group in groups:
                use(group)
            value = parse(line)
            if not value:
                continue
            use(value)


def prior_for_else_control(manager, groups, lines, parse, use):
    with manager:
        for line in lines:
            for group in groups:
                use(group)
            else:
                use(None)
            value = parse(line)
            if not value:
                continue
            use(value)


def prior_nested_for_control(manager, groups, others, lines, parse, use):
    with manager:
        for line in lines:
            for group in groups:
                for other in others:
                    use(group, other)
            value = parse(line)
            if not value:
                continue
            use(value)


def prior_nested_listcomp_control(manager, groups, others, lines, parse, use):
    with manager:
        for line in lines:
            use([(group, other) for group in groups for other in others])
            value = parse(line)
            if not value:
                continue
            use(value)


def in_loop_marker_control(manager, lines, parse, use, flag):
    with manager:
        for line in lines:
            if flag:
                use(line)
            value = parse(line)
            if not value:
                continue
            use(value)


def positive_name_control(manager, lines, parse, use):
    with manager:
        for line in lines:
            value = parse(line)
            if value:
                continue
            use(value)


def call_control(manager, lines, ready, use):
    with manager:
        for line in lines:
            if ready(line):
                continue
            use(line)


def negated_call_control(manager, lines, ready, use):
    with manager:
        for line in lines:
            if not ready(line):
                continue
            use(line)


def negated_method_control(manager, lines, use):
    with manager:
        for line in lines:
            if not line.ready():
                continue
            use(line)


def inner_try_control(manager, lines, parse, use):
    with manager:
        for line in lines:
            try:
                value = parse(line)
                if not value:
                    continue
                use(value)
            except ValueError:
                use(line)


def generator_control(manager, lines, parse, use):
    yield None
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


async def coroutine_control(manager, lines, parse, use, pause):
    await pause()
    with manager:
        for line in lines:
            value = parse(line)
            if not value:
                continue
            use(value)


async def async_for_control(manager, lines, parse, use):
    with manager:
        async for line in lines:
            value = await parse(line)
            if not value:
                continue
            use(value)


async def async_with_control(manager, lines, parse, use):
    async with manager:
        for line in lines:
            value = await parse(line)
            if not value:
                continue
            use(value)


def ownerless_control(lines, parse, use):
    for line in lines:
        value = parse(line)
        if not value:
            continue
        use(value)
";

    common::assert_matches_cpython_marshal(
        source,
        "<synchronous-with-iterator-negated-name-continue-compatibility>",
    );
}
