use crate::common;

#[test]
fn matches_cpython_module_super_symbol_admission() {
    for prefix in [
        "alias = super\n",
        "if False:\n    alias = super\n",
        "super = replacement\n",
        "from package import super\n",
        "global super\n",
        "def unrelated():\n    global super\n",
        "items = [super for item in values]\n",
        "def unrelated(value=super):\n    pass\n",
    ] {
        let source =
            format!("{prefix}class C:\n    def method(self):\n        return super().method()\n");
        common::assert_matches_cpython_marshal(&source, "module_super_symbol.py");
    }
}

#[test]
fn preserves_cpython_nested_super_symbol_scopes() {
    for prefix in [
        "",
        "def unrelated():\n    return super\n",
        "class Other:\n    alias = super\n",
        "factory = lambda: super\n",
        "items = (super for item in values)\n",
        "type Alias = super\n",
        "value: super\n",
        "def unrelated[T](value: super):\n    pass\n",
    ] {
        let source =
            format!("{prefix}class C:\n    def method(self):\n        return super().method()\n");
        common::assert_matches_cpython_marshal(&source, "nested_super_symbol.py");
    }
}
