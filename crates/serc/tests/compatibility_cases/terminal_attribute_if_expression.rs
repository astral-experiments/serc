use crate::common;

#[test]
fn matches_cpython_terminal_attribute_if_expression_joins() {
    for expression in ["1 if x else 2", "left if x else right"] {
        let assignment = format!("self.value = {expression}");
        for body in [
            format!("if flag:\n    {assignment}\nelse:\n    use()"),
            format!("if flag:\n    use()\nelse:\n    {assignment}"),
            format!("if flag:\n    {assignment}"),
            format!("if flag:\n    use()\nelif other:\n    {assignment}\nelse:\n    done()"),
            format!(
                "if flag:\n    if other:\n        {assignment}\n    else:\n        use()\nelse:\n    done()"
            ),
        ] {
            let source = format!(
                "def f(self, x, flag, other, left, right):\n    {}\n",
                body.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(&source, "terminal_attribute_if_expression.py");
        }
    }
}

#[test]
fn matches_cpython_attribute_if_expression_scope_controls() {
    for expression in [
        "(left if inner else right) if choose else other",
        "(left or right) if (choose and inner) else (other and fallback)",
    ] {
        let assignment = format!("self.value = {expression}");
        let tested = format!("if enabled:\n    {assignment}\nelse:\n    noop()");
        let otherwise = format!("if enabled:\n    noop()\nelse:\n    {assignment}");
        for body in [
            assignment,
            tested.clone(),
            otherwise.clone(),
            format!(
                "try:\n    {}\nexcept Error:\n    recover()",
                otherwise.replace('\n', "\n    ")
            ),
            format!(
                "try:\n    risky()\nexcept Error:\n    {}",
                tested.replace('\n', "\n    ")
            ),
            format!(
                "try:\n    risky()\nfinally:\n    {}",
                otherwise.replace('\n', "\n    ")
            ),
            format!("yield left\n{otherwise}"),
        ] {
            let source = format!(
                "def f(self, enabled, choose, inner, final, left, right, other, fallback, manager):\n    {}\n",
                body.replace('\n', "\n    "),
            );
            common::assert_matches_cpython_marshal(&source, "attribute_if_expression_scopes.py");
        }
        common::assert_matches_cpython_marshal(
            &format!("{otherwise}\n"),
            "attribute_if_expression_module.py",
        );
    }

    // These forms retain their existing terminal-assignment lowering.
    for source in [
        r"def f(self, x, flag, other, left, right):
    if flag:
        value = 1 if x else 2
    else:
        use()
",
        r"def f(self, x, flag, other, left, right):
    if flag:
        use()
    elif other:
        value = left if x else right
    else:
        done()
",
        r"def f(self, enabled, choose, inner, final, left, right, other, fallback, manager):
    if enabled:
        noop()
    else:
        self.value = other.value = (left if inner else right) if choose else other
",
        r"def f(self, enabled, choose, inner, final, left, right, other, fallback, manager):
    if enabled:
        noop()
    else:
        self.child.value = (left if inner else right) if choose else other
",
        r"def f(self, enabled, choose, inner, final, left, right, other, fallback, manager):
    if enabled:
        noop()
    else:
        with manager:
            self.value = (left or right) if (choose and inner) else (other and fallback)
",
        r"def f(choose, left, right):
    value: Type = left if choose else right
",
        r"def f(self, enabled, left):
    if enabled:
        self.value: Type = left
    else:
        noop()
",
    ] {
        common::assert_matches_cpython_marshal(source, "attribute_if_expression_controls.py");
    }
}
