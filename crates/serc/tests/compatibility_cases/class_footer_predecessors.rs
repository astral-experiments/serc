use crate::common;

#[test]
fn matches_cpython_class_footer_predecessors() {
    for prefix in [
        "",
        "def marker(self): return __class__\n",
        "\"docstring\"\n",
    ] {
        for body in [
            "if flag:\n    def f(self, value: int): ...\nelif other:\n    def g(self, value: int): ...\n",
            "if flag:\n    def f(self, value: int): ...\n    if other:\n        def g(self, value: int): ...\n",
        ] {
            let source = format!("class C:\n{}", common::indent(&format!("{prefix}{body}")));
            common::assert_matches_cpython_marshal(&source, "class_footer_predecessors.py");
        }
    }
}
