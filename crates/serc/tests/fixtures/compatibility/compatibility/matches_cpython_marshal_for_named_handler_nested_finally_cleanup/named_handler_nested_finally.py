class Container:
    def fallthrough_from_nested_try(self):
        try:
            body()
        except Error as __error:
            try:
                consume(__error)
            finally:
                pass
