class C(Base):
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
