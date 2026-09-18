use crate::common::assert_matches_cpython_marshal;

#[test]
fn loop_tail_source_entries_preserve_protected_condition_owners() {
    for (filename, source) in [
        (
            "loop_tail_elif_async_multiline_try_condition.py",
            r#"async def f(groups, sink, receiver, first, second):
    for group in groups:
        for item in group.items:
            if item.kind == KIND:
                sink.append(receiver.work(
                    first,
                    second,
                    key=item.key,
                    value=item.extra.get("value"),
                ))
                try:
                    if item.extra and "value" in item.extra:
                        await consume(item)
                except Exception:
                    recover()
            elif item.kind == OTHER:
                sink.append(receiver.other(first))
"#,
        ),
        (
            "loop_tail_elif_local_call_try_condition.py",
            r#"def f(items, callback):
    for recipient in items:
        if recipient.enabled:
            callback(TOKEN)
            try:
                if recipient.name.startswith("x"):
                    consume(recipient)
            except Exception:
                recover()
        elif recipient.other:
            callback(OTHER)
"#,
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn specialized_super_records_preserve_cleanup_owners() {
    for (filename, source) in [
        (
            "super_1_list_0_4_fusions.py",
            r"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super().prepare(a)
            resource = work(a, b)
            value = work(a, b)
            value = work(a, b)
            value = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
",
        ),
        (
            "super_1_list_1_1_fusions.py",
            r#"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super().prepare(a)
            self.audit(["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"])
            resource = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
"#,
        ),
        (
            "super_1_list_1_4_fusions.py",
            r#"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super().prepare(a)
            self.audit(["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"])
            resource = work(a, b)
            value = work(a, b)
            value = work(a, b)
            value = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
"#,
        ),
        (
            "explicit_super_control.py",
            r#"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super(C, self).prepare(a)
            self.audit(["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"])
            resource = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
"#,
        ),
        (
            "attribute_super_control.py",
            r#"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            value = super().value
            self.audit(["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"])
            resource = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
"#,
        ),
        (
            "nonliteral_list_control.py",
            r"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super().prepare(a)
            self.audit([a, b, a, b, a, b])
            resource = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
",
        ),
        (
            "super_1_list_0_1_fusions.py",
            r"class C(Base):
    def f(self, a, b):
        resource = None
        try:
            super().prepare(a)
            resource = work(a, b)
        finally:
            if resource is not None:
                resource.close()
            self.finish()
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
