class C(Base):
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
