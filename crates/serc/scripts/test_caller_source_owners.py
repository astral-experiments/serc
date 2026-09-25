"""Compile-only tests; none of the sample modules is executed."""

import textwrap
import unittest

from caller_source_owners import derive_retained_code


def derive(source, module="toy"):
    source = textwrap.dedent(source)
    return derive_retained_code(source, compile(source, "<owner-test>", "exec"), module)


class RetainedCodeTests(unittest.TestCase):
    def test_direct_function_owns_nested_code_but_not_module_constants(self):
        result = derive("""
            module_value = "module_only_marker"
            def survivor(argument):
                def never_called():
                    return "nested_owned_marker"
                return "function_owned_marker"
        """)
        self.assertIn("function_owned_marker", result["strings"])
        self.assertIn("nested_owned_marker", result["strings"])
        self.assertIn("argument", result["strings"])
        self.assertNotIn("module_only_marker", result["strings"])
        self.assertEqual(len(result["owners"]), 1)

    def test_class_owns_methods_not_class_suite(self):
        result = derive("""
            class Plain:
                class_value = "class_only_marker"
                def method(self):
                    return "method_owned_marker"
        """)
        self.assertIn("method_owned_marker", result["strings"])
        self.assertNotIn("class_only_marker", result["strings"])
        self.assertEqual(result["owners"][0]["owner"], "toy.Plain.method")

    def test_replaced_deleted_and_conditional_roots_are_not_owners(self):
        result = derive("""
            def replaced(): return "replaced_marker"
            def deleted(): return "deleted_marker"
            def conditional(): return "conditional_marker"
            replaced = None
            del deleted
            if False:
                conditional = None
            def survivor(): return "surviving_marker"
        """)
        self.assertEqual(
            [owner["owner"] for owner in result["owners"]], ["toy.survivor"]
        )
        self.assertNotIn("replaced_marker", result["strings"])
        self.assertNotIn("deleted_marker", result["strings"])
        self.assertNotIn("conditional_marker", result["strings"])
        self.assertTrue(result["unresolved"])

    def test_property_setter_retains_getter_and_new_setter(self):
        result = derive("""
            class Plain:
                @property
                def value(self): return "getter_owned_marker"
                @value.setter
                def value(self, item): self.item = "setter_owned_marker"
        """)
        self.assertIn("getter_owned_marker", result["strings"])
        self.assertIn("setter_owned_marker", result["strings"])
        self.assertEqual(len(result["owners"]), 2)

    def test_property_getter_replacement_drops_old_getter(self):
        result = derive("""
            class Plain:
                @property
                def value(self): return "old_getter_marker"
                @value.getter
                def value(self): return "new_getter_marker"
        """)
        self.assertNotIn("old_getter_marker", result["strings"])
        self.assertIn("new_getter_marker", result["strings"])

    def test_known_wrappers_and_aliases_retain_functions(self):
        result = derive("""
            from abc import abstractmethod as abstract
            from functools import lru_cache as cache
            from reprlib import recursive_repr as recursive
            @cache(maxsize=4)
            def cached(): return "cached_owned_marker"
            class Plain:
                @classmethod
                @abstract
                def method(cls): return "class_method_marker"
                @staticmethod
                def static(): return "static_method_marker"
                @recursive()
                def __repr__(self): return "recursive_marker"
        """)
        for marker in (
            "cached_owned_marker",
            "class_method_marker",
            "static_method_marker",
            "recursive_marker",
        ):
            self.assertIn(marker, result["strings"])

    def test_unknown_eager_mutation_invalidates_all_positive_claims(self):
        result = derive("""
            def survivor(): return "not_proved_marker"
            mutate_globals()
        """)
        self.assertEqual(result["strings"], [])
        self.assertEqual(result["owners"], [])
        self.assertTrue(result["unresolved"])

    def test_default_walrus_replaces_prior_root(self):
        result = derive("""
            def replaced(): return "replaced_marker"
            def survivor(value=(replaced := None)): return "surviving_marker"
        """)
        self.assertNotIn("replaced_marker", result["strings"])
        self.assertIn("surviving_marker", result["strings"])

    def test_private_method_alias_replacement_uses_mangled_binding(self):
        result = derive("""
            class Plain:
                def __method(self): return "private_replaced_marker"
                _Plain__method = None
        """)
        self.assertNotIn("private_replaced_marker", result["strings"])

    def test_conditional_default_walrus_may_replace_prior_root(self):
        result = derive("""
            def replaced(): return "conditional_default_marker"
            if condition:
                def conditional(value=(replaced := None)): pass
        """)
        self.assertNotIn("conditional_default_marker", result["strings"])

    def test_assigned_subclass_hook_prevents_plain_class_proof(self):
        result = derive("""
            class Base:
                @classmethod
                def hook(cls): del cls.method
                __init_subclass__ = hook
            class Child(Base):
                def method(self): return "deleted_by_hook_marker"
        """)
        self.assertNotIn("deleted_by_hook_marker", result["strings"])
        self.assertTrue(result["unresolved"])

    def test_attribute_write_excludes_owner_and_valueless_annotation_does_not(self):
        result = derive("""
            def changed(): return "changed_marker"
            def survivor(): return "surviving_marker"
            changed.__code__ = replacement
            survivor: object
        """)
        self.assertNotIn("changed_marker", result["strings"])
        self.assertIn("surviving_marker", result["strings"])

    def test_mutating_decorator_namespace_invalidates_identity_proof(self):
        result = derive("""
            import builtins
            def discard(function): return None
            builtins.property = discard
            class Plain:
                @property
                def value(self): return "discarded_property_marker"
        """)
        self.assertNotIn("discarded_property_marker", result["strings"])

    def test_local_class_alias_cannot_reuse_shadowed_outer_base_proof(self):
        result = derive("""
            class Safe: pass
            class Evil:
                def __init_subclass__(cls): del cls.method
            class Outer:
                Safe = Evil
                class Child(Safe):
                    def method(self): return "hook_deleted_marker"
        """)
        self.assertNotIn("hook_deleted_marker", result["strings"])

    def test_class_suite_mutation_of_outer_owner_fails_closed(self):
        result = derive("""
            class First:
                def method(self): return "outer_deleted_marker"
            class Second:
                First.method = None
        """)
        self.assertNotIn("outer_deleted_marker", result["strings"])


if __name__ == "__main__":
    unittest.main()
