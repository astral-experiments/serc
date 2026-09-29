class Doc:
    def document(self, object, name=None, *args):
        """Generate documentation for an object."""
        args = (object, name) + args
        # 'try' clause is to attempt to handle the possibility that inspect
        # identifies something in a way that pydoc itself has issues handling;
        # think 'super' and how it is a descriptor (which raises the exception
        # by lacking a __name__ attribute) and an instance.
        try:
            if inspect.ismodule(object): return self.docmodule(*args)
            if inspect.isclass(object): return self.docclass(*args)
            if inspect.isroutine(object): return self.docroutine(*args)
        except AttributeError:
            pass
        if inspect.isdatadescriptor(object): return self.docdata(*args)
        return self.docother(*args)
