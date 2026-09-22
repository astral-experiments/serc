use crate::common;

#[test]
fn terminal_generator_breaks_leave_the_enclosing_try_after_handler_cleanup() {
    for (handler, binding) in [("unnamed", ""), ("named", " as error")] {
        for (tail, continuation) in [("terminal", ""), ("nonterminal", "        print(items)\n")] {
            let source = format!(
                "def f(items):
    try:
        while True:
            try:
                item = next(items)
            except StopIteration{binding}:
                yield None
                break
            yield item
{continuation}    except StopIteration:
        pass
"
            );
            common::assert_matches_cpython_marshal(
                &source,
                &format!("generator_break_{handler}_{tail}.py"),
            );
        }
    }
}
